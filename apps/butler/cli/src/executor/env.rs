//! A task's environment, as nx builds it: butler's own environment plus the
//! dotenv files nx loads per task (`getEnvPathsForTask`, dotenv 16 +
//! dotenv-expand 12 semantics). Values already in the environment win over
//! every file, and among files the first listed wins — so `.env.local`
//! overrides `.env`, and a project's files override the workspace's.

use std::collections::BTreeMap;
use std::path::Path;

use eyre::{Result, bail};

/// The dotenv files nx consults for a task, most specific first
/// (`task-env-paths.js`).
pub fn dotenv_paths(project_root: &str, target: &str, configuration: Option<&str>) -> Vec<String> {
    let mut identifiers = Vec::new();
    if let Some(c) = configuration {
        identifiers.push(format!("{target}.{c}"));
        identifiers.push(c.to_string());
    }
    identifiers.push(target.to_string());
    identifiers.push(String::new());
    let variants = |id: &str, root: Option<&str>| -> Vec<String> {
        let prefix = root.map(|r| format!("{r}/")).unwrap_or_default();
        if id.is_empty() {
            vec![
                format!("{prefix}.env.local"),
                format!("{prefix}.local.env"),
                format!("{prefix}.env"),
            ]
        } else {
            vec![
                format!("{prefix}.env.{id}.local"),
                format!("{prefix}.env.{id}"),
                format!("{prefix}.{id}.local.env"),
                format!("{prefix}.{id}.env"),
            ]
        }
    };
    let mut out = Vec::new();
    for id in &identifiers {
        out.extend(variants(id, Some(project_root)));
    }
    for id in &identifiers {
        out.extend(variants(id, None));
    }
    out
}

/// `base` (butler's environment) plus the task's dotenv files, unless
/// `NX_LOAD_DOT_ENV_FILES=false`.
pub fn task_env(
    workspace_root: &Path,
    base: &BTreeMap<String, String>,
    project_root: &str,
    target: &str,
    configuration: Option<&str>,
) -> Result<BTreeMap<String, String>> {
    let mut env = base.clone();
    if env.get("NX_LOAD_DOT_ENV_FILES").map(String::as_str) == Some("false") {
        return Ok(env);
    }
    let files: Vec<_> = dotenv_paths(project_root, target, configuration)
        .into_iter()
        .map(|f| workspace_root.join(f))
        .collect();
    load_and_expand(&files, &mut env)?;
    Ok(env)
}

/// dotenv `config({path: files, processEnv: env, override: false})` then
/// dotenv-expand. Missing files are skipped, as dotenv does.
pub fn load_and_expand(
    files: &[impl AsRef<Path>],
    env: &mut BTreeMap<String, String>,
) -> Result<()> {
    // Across files, the first value set for a key wins.
    let mut parsed: Vec<(String, String)> = Vec::new();
    for file in files {
        let Ok(src) = std::fs::read_to_string(file.as_ref()) else {
            continue;
        };
        for (k, v) in parse(&src) {
            if !parsed.iter().any(|(pk, _)| *pk == k) {
                parsed.push((k, v));
            }
        }
    }
    for (k, v) in &parsed {
        env.entry(k.clone()).or_insert_with(|| v.clone());
    }
    expand(&mut parsed, env)?;
    for (k, v) in parsed {
        env.insert(k, v);
    }
    Ok(())
}

/// dotenv 16 `parse`: `KEY=value` / `KEY: value` lines, optional `export`,
/// single/double/backtick quotes (multi-line allowed), `#` comments; double
/// quotes expand `\n`/`\r`. A repeated key takes the last value.
pub fn parse(src: &str) -> Vec<(String, String)> {
    let text = src.replace("\r\n", "\n").replace('\r', "\n");
    let bytes = text.as_bytes();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut line_start = 0;
    while line_start < bytes.len() {
        let next_line = |from: usize| {
            text[from..]
                .find('\n')
                .map_or(bytes.len(), |i| from + i + 1)
        };
        match parse_entry(&text, line_start) {
            Some((key, value, end)) => {
                out.retain(|(k, _)| *k != key);
                out.push((key, value));
                // The regex resumes right after the match; the next match
                // must begin at a line start.
                line_start = if end > 0 && bytes[end - 1] == b'\n' {
                    end
                } else {
                    next_line(end)
                };
            }
            None => line_start = next_line(line_start),
        }
    }
    out
}

/// One `LINE` regex match starting at `start` (a line start).
fn parse_entry(text: &str, start: usize) -> Option<(String, String, usize)> {
    let b = text.as_bytes();
    let mut i = start;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if text[i..].starts_with("export") && b.get(i + 6).is_some_and(u8::is_ascii_whitespace) {
        let mut j = i + 6;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        // `export` might itself be the key (`export=1`); only treat it as the
        // keyword when a key follows.
        if b.get(j).is_some_and(|c| is_key_byte(*c)) {
            i = j;
        }
    }
    let key_start = i;
    while i < b.len() && is_key_byte(b[i]) {
        i += 1;
    }
    if i == key_start {
        return None;
    }
    let key = text[key_start..i].to_string();
    // `\s*=\s*?` or `:\s+?`
    let mut j = i;
    while j < b.len() && b[j].is_ascii_whitespace() && b[j] != b'\n' {
        j += 1;
    }
    let after_sep = if b.get(j) == Some(&b'=') {
        j + 1
    } else if b.get(i) == Some(&b':') && b.get(i + 1).is_some_and(u8::is_ascii_whitespace) {
        i + 1
    } else {
        return None;
    };
    // Value alternatives, each allowing leading whitespace.
    let mut k = after_sep;
    while k < b.len() && (b[k] == b' ' || b[k] == b'\t') {
        k += 1;
    }
    if let Some(&q) = b.get(k)
        && matches!(q, b'\'' | b'"' | b'`')
        && let Some(close) = find_closing_quote(b, k + 1, q)
    {
        let raw = &text[after_sep..=close];
        if let Some(end) = line_tail(b, close + 1) {
            return Some((key, clean_value(raw), end));
        }
    }
    let mut v = after_sep;
    while v < b.len() && b[v] != b'#' && b[v] != b'\n' {
        v += 1;
    }
    let raw = &text[after_sep..v];
    let end = line_tail(b, v)?;
    Some((key, clean_value(raw), end))
}

fn is_key_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-')
}

/// A closing quote, skipping backslash-escaped ones (`(?:\\q|[^q])*`).
fn find_closing_quote(b: &[u8], mut i: usize, q: u8) -> Option<usize> {
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1) == Some(&q) {
            i += 2;
        } else if b[i] == q {
            return Some(i);
        } else {
            i += 1;
        }
    }
    None
}

/// `\s*(?:#.*)?$`: trailing blanks and an optional comment up to end of line.
fn line_tail(b: &[u8], mut i: usize) -> Option<usize> {
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if b.get(i) == Some(&b'#') {
        while i < b.len() && b[i] != b'\n' {
            i += 1;
        }
    }
    match b.get(i) {
        None => Some(i),
        Some(b'\n') => Some(i + 1),
        Some(_) => None,
    }
}

fn clean_value(raw: &str) -> String {
    let v = raw.trim();
    let quote = v.chars().next();
    let unquoted = match quote {
        Some(q @ ('\'' | '"' | '`')) if v.len() >= 2 && v.ends_with(q) => &v[1..v.len() - 1],
        _ => v,
    };
    if quote == Some('"') {
        unquoted.replace("\\n", "\n").replace("\\r", "\r")
    } else {
        unquoted.to_string()
    }
}

/// dotenv-expand 12 `expand`: `${VAR}`, `$VAR`, `${VAR:-d}`/`-`/`:+`/`+`,
/// `\$` escapes; the environment wins over file values.
fn expand(parsed: &mut [(String, String)], env: &mut BTreeMap<String, String>) -> Result<()> {
    let mut running: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in parsed.iter_mut() {
        let resolved = match env.get(key.as_str()) {
            Some(existing) if !existing.is_empty() && existing != value => existing.clone(),
            _ => expand_value(key, value, env, &running)?,
        };
        let unescaped = resolved.replace("\\$", "$");
        running.insert(key.clone(), unescaped.clone());
        *value = unescaped;
    }
    for (k, v) in parsed.iter() {
        env.insert(k.clone(), v.clone());
    }
    Ok(())
}

fn expand_value(
    key: &str,
    value: &str,
    env: &BTreeMap<String, String>,
    running: &BTreeMap<String, String>,
) -> Result<String> {
    let lookup = |k: &str| env.get(k).or_else(|| running.get(k)).cloned();
    let mut result = value.to_string();
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..10_000 {
        let Some((start, end, expr)) = next_reference(&result) else {
            return Ok(result);
        };
        seen.push(result.clone());
        let template = result[start..end].to_string();
        let splitter = [":+", "+", ":-", "-"]
            .iter()
            .filter_map(|op| expr.find(op).map(|i| (i, *op)))
            .min_by_key(|(i, op)| (*i, std::cmp::Reverse(op.len())))
            .map(|(_, op)| op);
        let (name, rest) = match splitter {
            Some(op) => {
                let mut parts = expr.split(op);
                let name = parts.next().unwrap_or_default().to_string();
                (name, parts.collect::<Vec<_>>().join(op))
            }
            None => (expr.clone(), String::new()),
        };
        let (default, current) = if matches!(splitter, Some(":+" | "+")) {
            let set = lookup(&name).is_some_and(|v| !v.is_empty());
            (if set { rest } else { String::new() }, None)
        } else {
            (rest, lookup(&name).filter(|v| !v.is_empty()))
        };
        let replacement = match current {
            Some(v) if seen.contains(&v) => default,
            Some(v) => v,
            None => default,
        };
        result = result.replacen(&template, &replacement, 1);
        if running.get(&name).is_some_and(|r| *r == result) {
            return Ok(result);
        }
    }
    bail!("dotenv: expanding `{key}` does not terminate")
}

/// First `${expr}` / `$NAME` not preceded by a backslash: (start, end, expr).
fn next_reference(s: &str) -> Option<(usize, usize, String)> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' && (i == 0 || b[i - 1] != b'\\') {
            if b.get(i + 1) == Some(&b'{')
                && let Some(close) = s[i + 2..].find(['{', '}'])
                && b[i + 2 + close] == b'}'
                && close > 0
            {
                return Some((i, i + 3 + close, s[i + 2..i + 2 + close].to_string()));
            }
            if b.get(i + 1)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
            {
                let mut j = i + 2;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                return Some((i, j, s[i + 1..j].to_string()));
            }
        }
        i += 1;
    }
    None
}

/// npm-run-path 4 (what run-commands puts in front of `PATH`): every
/// `node_modules/.bin` from `cwd` up to the filesystem root, skipping ones
/// already on `PATH`. (It also appends the running node binary's directory;
/// butler has none, and that directory is on `PATH` already whenever
/// `bun`/`node` resolve.)
pub fn npm_run_path(cwd: &Path, path: &str) -> String {
    let parts: Vec<&str> = path.split(':').collect();
    let mut result: Vec<String> = Vec::new();
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let bin = d.join("node_modules/.bin").to_string_lossy().into_owned();
        if !parts.contains(&bin.as_str()) {
            result.push(bin);
        }
        dir = d.parent();
    }
    if path.is_empty() || path == ":" {
        format!("{}{path}", result.join(":"))
    } else {
        result.push(path.to_string());
        result.join(":")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_parse_rules() {
        let src = "# c\nexport A=1\nB = two words # comment\nC=\"x\\ny\"\nD='keep \\n'\nE=\nF: colon\nbad line\nB=again\nG=\"multi\nline\"\n";
        let parsed: BTreeMap<_, _> = parse(src).into_iter().collect();
        assert_eq!(parsed["A"], "1");
        assert_eq!(parsed["B"], "again");
        assert_eq!(parsed["C"], "x\ny");
        assert_eq!(parsed["D"], "keep \\n");
        assert_eq!(parsed["E"], "");
        assert_eq!(parsed["F"], "colon");
        assert_eq!(parsed["G"], "multi\nline");
        assert!(!parsed.contains_key("bad"));
    }

    #[test]
    fn env_wins_first_file_wins_and_expansion() {
        let dir = std::env::temp_dir().join(format!("butler-dotenv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.env"),
            "X=from-a\nY=${HOME_ISH}/y\nZ=${MISSING:-dflt}\n",
        )
        .unwrap();
        std::fs::write(dir.join("b.env"), "X=from-b\nW=$X-w\nP=preset-file\n").unwrap();
        let mut env = BTreeMap::from([
            ("HOME_ISH".to_string(), "/h".to_string()),
            ("P".to_string(), "preset".to_string()),
        ]);
        load_and_expand(
            &[
                dir.join("a.env"),
                dir.join("missing.env"),
                dir.join("b.env"),
            ],
            &mut env,
        )
        .unwrap();
        assert_eq!(env["X"], "from-a");
        assert_eq!(env["Y"], "/h/y");
        assert_eq!(env["Z"], "dflt");
        assert_eq!(env["W"], "from-a-w");
        assert_eq!(env["P"], "preset");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dotenv_paths_are_most_specific_first() {
        let p = dotenv_paths("apps/x", "build", Some("ci"));
        assert_eq!(p[0], "apps/x/.env.build.ci.local");
        assert_eq!(
            p.iter().position(|f| f == "apps/x/.env").unwrap() + 1,
            p.iter().position(|f| f == ".env.build.ci.local").unwrap()
        );
        assert_eq!(p.last().unwrap(), ".env");
    }

    #[test]
    fn npm_run_path_prefixes_local_bins() {
        let p = npm_run_path(Path::new("/w/apps/x"), "/usr/bin:/w/node_modules/.bin");
        assert_eq!(
            p,
            "/w/apps/x/node_modules/.bin:/w/apps/node_modules/.bin:/node_modules/.bin:/usr/bin:/w/node_modules/.bin"
        );
    }
}
