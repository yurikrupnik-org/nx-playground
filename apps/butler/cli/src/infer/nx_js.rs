//! The edges nx's built-in JavaScript plugin (`nx/js/dependencies-and-lockfile`)
//! adds to every workspace: a project depends on another when its
//! `package.json` names that project's package, or when one of its source
//! files imports it.
//!
//! Module resolution is where butler departs from nx, deliberately and
//! narrowly. nx resolves a bare import through tsconfig `paths`, TypeScript's
//! resolver and Node's `require.resolve` — which all end in the same place for
//! a workspace member, because the package manager links every member into
//! `node_modules`. butler resolves a bare import against those members
//! directly (their `exports` entry points, or their files when a package has
//! no `exports`) and against the root tsconfig's `paths`; relative imports
//! resolve by path. External packages (`npm:` nodes) are not modelled.

use std::collections::BTreeMap;
use std::path::Path;

use eyre::Result;

use super::nx_core::{PackageFile, package_files, read_json};
use super::{Ctx, DepKind, Dependency, Layer, Node, ProjectRoots, normalize_root};
use crate::config::Json;

/// nx's `moduleExtensions`: the files whose imports are analyzed.
const MODULE_EXTENSIONS: &[&str] = &[
    ".ts", ".js", ".tsx", ".jsx", ".mts", ".mjs", ".cjs", ".cts", ".vue",
];

pub struct JsLayer;

impl Layer for JsLayer {
    fn name(&self) -> &str {
        "nx/js/dependencies-and-lockfile"
    }

    /// nx's JS plugin creates only external (lock-file) nodes, which butler
    /// does not model.
    fn nodes(&self, _ctx: &Ctx) -> Result<Vec<Node>> {
        Ok(Vec::new())
    }

    fn dependencies(&self, ctx: &Ctx, projects: &ProjectRoots) -> Result<Vec<Dependency>> {
        let config = JsConfig::read(ctx)?;
        let locator = Locator::new(ctx, projects)?;
        let mut out = Vec::new();
        if config.analyze_source_files && typescript_resolves(ctx.workspace_root) {
            out.extend(locator.import_dependencies(ctx)?);
        }
        if config.analyze_package_json {
            out.extend(locator.package_json_dependencies(ctx)?);
        }
        Ok(out)
    }
}

/// nx analyzes source files only when `require.resolve('typescript')`
/// succeeds from nx's own package — Node's lookup through every
/// `node_modules` above it, which under bun's or pnpm's isolated layout
/// includes the store's hoisted `node_modules`, not just the workspace's.
fn typescript_resolves(workspace_root: &Path) -> bool {
    let start = std::fs::canonicalize(workspace_root.join("node_modules/nx"))
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    start
        .ancestors()
        .filter(|d| d.file_name() != Some("node_modules".as_ref()))
        .any(|d| d.join("node_modules/typescript/package.json").is_file())
}

/// nx's `jsPluginConfig`: `pluginsConfig["@nx/js"]` when set, else source
/// analysis only in a workspace that depends on an nx JS-flavoured package.
struct JsConfig {
    analyze_package_json: bool,
    analyze_source_files: bool,
}

impl JsConfig {
    fn read(ctx: &Ctx) -> Result<Self> {
        let nx: Json = if ctx.workspace_root.join("nx.json").is_file() {
            read_json(ctx, "nx.json")?
        } else {
            Json::Null
        };
        if let Some(cfg) = nx.pointer("/pluginsConfig/@nx~1js") {
            let flag = |k: &str| cfg.get(k).and_then(Json::as_bool).unwrap_or(true);
            return Ok(Self {
                analyze_package_json: flag("analyzePackageJson"),
                analyze_source_files: flag("analyzeSourceFiles"),
            });
        }
        if !ctx.workspace_root.join("package.json").is_file() {
            return Ok(Self {
                analyze_package_json: false,
                analyze_source_files: false,
            });
        }
        let root: Json = read_json(ctx, "package.json")?;
        let has = |dep: &str| {
            ["dependencies", "devDependencies"]
                .iter()
                .any(|section| root.get(section).and_then(|s| s.get(dep)).is_some())
        };
        Ok(Self {
            analyze_package_json: true,
            analyze_source_files: [
                "@nx/workspace",
                "@nx/js",
                "@nx/node",
                "@nx/next",
                "@nx/react",
                "@nx/angular",
                "@nx/web",
            ]
            .into_iter()
            .any(has),
        })
    }
}

/// A workspace package, as nx's `metadata.js` records it.
struct Package {
    project: String,
    root: String,
    version: Option<String>,
    in_workspaces: bool,
    exports: Option<Json>,
    main: bool,
}

/// nx's `TargetProjectLocator`, reduced to workspace projects.
struct Locator {
    /// root -> project name.
    roots: BTreeMap<String, String>,
    /// npm name -> package (nx's `packageToProjectMap`, later wins).
    packages: BTreeMap<String, Package>,
    /// Root tsconfig `compilerOptions.paths`.
    paths: BTreeMap<String, Vec<String>>,
}

impl Locator {
    fn new(ctx: &Ctx, projects: &ProjectRoots) -> Result<Self> {
        let roots: BTreeMap<String, String> = projects
            .iter()
            .map(|(name, root)| (root.clone(), name.clone()))
            .collect();
        let mut packages = BTreeMap::new();
        for PackageFile {
            root,
            in_workspaces,
            json,
            ..
        } in package_files(ctx)?
        {
            let root = json
                .nx
                .as_ref()
                .and_then(|n| n.root.as_deref())
                .map_or(root, normalize_root);
            let (Some(npm), Some(project)) = (json.name.clone(), roots.get(&root)) else {
                continue;
            };
            packages.insert(
                npm,
                Package {
                    project: project.clone(),
                    root,
                    version: json.version.clone(),
                    in_workspaces,
                    exports: json.exports.clone(),
                    main: json.main.is_some(),
                },
            );
        }
        Ok(Self {
            roots,
            packages,
            paths: root_tsconfig_paths(ctx)?,
        })
    }

    /// nx's `findProjectForPath`: the deepest project root containing `path`.
    fn project_for_path(&self, path: &str) -> Option<&String> {
        let mut p = normalize_root(path);
        loop {
            if let Some(name) = self.roots.get(&p) {
                return Some(name);
            }
            if p == "." {
                return None;
            }
            p = p
                .rsplit_once('/')
                .map_or(".".to_string(), |(d, _)| d.to_string());
        }
    }

    /// nx's `findProjectOfResolvedModule`.
    fn project_of_resolved(&self, path: &str) -> Option<&String> {
        if path.starts_with("node_modules/") || path.contains("/node_modules/") {
            return None;
        }
        self.project_for_path(path)
    }

    /// `package.json` dependency edges (`buildExplicitPackageJsonDependencies`):
    /// a dependency naming a workspace package whose version the range
    /// accepts (`workspace:`, `*`, a matching `file:` path, or semver).
    fn package_json_dependencies(&self, ctx: &Ctx) -> Result<Vec<Dependency>> {
        let listed: std::collections::BTreeSet<&str> = ctx
            .files
            .iter()
            .map(String::as_str)
            .filter(|f| f.ends_with("package.json"))
            .collect();
        let mut out = Vec::new();
        for (root, source) in &self.roots {
            let file = if root == "." {
                "package.json".to_string()
            } else {
                format!("{root}/package.json")
            };
            if !listed.contains(file.as_str()) || !ctx.workspace_root.join(&file).is_file() {
                continue;
            }
            let Ok(json) = read_json::<Json>(ctx, &file) else {
                // nx skips a manifest it cannot parse here.
                continue;
            };
            // Later sections win, production dependencies most important.
            let mut deps: BTreeMap<&str, (&Json, &str)> = BTreeMap::new();
            for section in [
                "optionalDependencies",
                "peerDependencies",
                "devDependencies",
                "dependencies",
            ] {
                for (name, version) in json
                    .get(section)
                    .and_then(Json::as_object)
                    .into_iter()
                    .flatten()
                {
                    deps.insert(name, (version, section));
                }
            }
            let dev_only = |name: &str| {
                ["optionalDependencies", "peerDependencies", "dependencies"]
                    .iter()
                    .all(|s| json.get(s).and_then(|d| d.get(name)).is_none())
            };
            for (name, (version, _)) in deps {
                let Some(version) = version.as_str() else {
                    continue;
                };
                if let Some(target) = self.workspace_dependency(&file, name, version) {
                    out.push(Dependency {
                        source: source.clone(),
                        target,
                        kind: DepKind::Static,
                        source_file: Some(file.clone()),
                        dev: dev_only(name),
                    });
                }
            }
        }
        Ok(out)
    }

    /// nx's `findDependencyInWorkspaceProjects`.
    fn workspace_dependency(&self, manifest: &str, name: &str, range: &str) -> Option<String> {
        let pkg = self.packages.get(name)?;
        let workspace = range.starts_with("workspace:");
        let range = range.strip_prefix("workspace:").unwrap_or(range);
        if workspace || range == "*" {
            return Some(pkg.project.clone());
        }
        if let Some(path) = range.strip_prefix("file:") {
            let dir = manifest.rsplit_once('/').map_or("", |(d, _)| d);
            if join_path(dir, path).as_deref() == Some(pkg.root.as_str()) {
                return Some(pkg.project.clone());
            }
        }
        let version = pkg.version.as_deref()?;
        semver::satisfies(version, range).then(|| pkg.project.clone())
    }

    /// Source-import edges (`buildExplicitTypeScriptDependencies`).
    fn import_dependencies(&self, ctx: &Ctx) -> Result<Vec<Dependency>> {
        let mut out = Vec::new();
        for file in ctx
            .files
            .iter()
            .filter(|f| MODULE_EXTENSIONS.iter().any(|ext| f.ends_with(ext)))
        {
            let Some(source) = self.project_for_path(file) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(ctx.workspace_root.join(file)) else {
                continue;
            };
            let source_is_root = self.roots.get(".") == Some(source);
            for (spec, kind) in imports::find(&text) {
                let Some(target) = self.resolve(ctx.workspace_root, &spec, file) else {
                    continue;
                };
                // nx keeps edges into the root project only from the root
                // project itself (its config files would otherwise tie every
                // project to it).
                if !source_is_root && self.roots.get(".") == Some(&target) {
                    continue;
                }
                out.push(Dependency {
                    source: source.clone(),
                    target,
                    kind,
                    source_file: Some(file.clone()),
                    dev: false,
                });
            }
        }
        Ok(out)
    }

    /// nx's `findProjectFromImport`, for workspace projects.
    fn resolve(&self, workspace_root: &Path, spec: &str, file: &str) -> Option<String> {
        if spec == "." || spec == ".." || spec.starts_with("./") || spec.starts_with("../") {
            let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
            return self.project_of_resolved(&join_path(dir, spec)?).cloned();
        }
        if let Some(found) = self.resolve_paths(spec) {
            return Some(found);
        }
        let name = package_name(spec);
        let pkg = self.packages.get(name).filter(|p| p.in_workspaces)?;
        let hit = match &pkg.exports {
            Some(exports) => exported(name, exports, spec),
            None if spec == name => {
                pkg.main
                    || ["index.js", "index.json", "index.node"]
                        .iter()
                        .any(|f| workspace_root.join(&pkg.root).join(f).is_file())
            }
            None => {
                let sub = workspace_root.join(&pkg.root).join(&spec[name.len() + 1..]);
                let with = |ext: &str| {
                    let mut s = sub.clone().into_os_string();
                    s.push(ext);
                    std::path::PathBuf::from(s).is_file()
                };
                sub.is_file()
                    || [".js", ".json", ".node"].into_iter().any(with)
                    || ["index.js", "index.json", "index.node"]
                        .iter()
                        .any(|f| sub.join(f).is_file())
            }
        };
        hit.then(|| pkg.project.clone())
    }

    /// The root tsconfig's `paths`: an exact key, else the pattern with the
    /// longest prefix; the first mapped location inside a project wins.
    fn resolve_paths(&self, spec: &str) -> Option<String> {
        let (key, star) = if self.paths.contains_key(spec) {
            (spec, None)
        } else {
            self.paths
                .keys()
                .filter_map(|k| {
                    let (prefix, suffix) = k.split_once('*')?;
                    (!suffix.contains('*')
                        && spec.len() >= prefix.len() + suffix.len()
                        && spec.starts_with(prefix)
                        && spec.ends_with(suffix))
                    .then(|| {
                        (
                            k.as_str(),
                            prefix.len(),
                            &spec[prefix.len()..spec.len() - suffix.len()],
                        )
                    })
                })
                .max_by_key(|(_, len, _)| *len)
                .map(|(k, _, s)| (k, Some(s)))?
        };
        for target in &self.paths[key] {
            let path = match star {
                Some(s) => target.replacen('*', s, 1),
                None => target.clone(),
            };
            if let Some(p) = self.project_of_resolved(path.trim_start_matches("./")) {
                return Some(p.clone());
            }
        }
        None
    }
}

/// nx's `getPackageNameFromImportPath`.
fn package_name(spec: &str) -> &str {
    let mut parts = spec.splitn(3, '/');
    let first = parts.next().unwrap_or("");
    if first.starts_with('@')
        && let Some(second) = parts.next()
    {
        return &spec[..first.len() + 1 + second.len()];
    }
    first
}

/// Whether `spec` is one of the entry points a package's `exports` declares
/// (nx's `getWorkspacePackagesMetadata` + wildcard matching).
fn exported(name: &str, exports: &Json, spec: &str) -> bool {
    let map = match exports {
        Json::String(_) => return spec == name,
        Json::Object(m) => m,
        _ => return false,
    };
    map.iter().filter(|(_, v)| !v.is_null()).any(|(key, _)| {
        let Some(sub) = key.strip_prefix('.') else {
            // Conditional exports: the package name is the entry point.
            return spec == name;
        };
        let entry = format!("{name}{sub}");
        let entry = entry.trim_end_matches('/');
        match entry.split('*').collect::<Vec<_>>()[..] {
            [_] => spec == entry,
            [base, trailer] => {
                spec != base
                    && spec.starts_with(base)
                    && (trailer.is_empty()
                        || (spec.ends_with(trailer) && spec.len() >= entry.len()))
            }
            _ => false,
        }
    })
}

/// `posix.join(dir, rel)`, normalized; `None` when it climbs above the
/// workspace root.
fn join_path(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for seg in dir.split('/').chain(rel.split('/')) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}

/// `compilerOptions.paths` of `tsconfig.base.json` (else `tsconfig.json`) at
/// the workspace root — the only tsconfig nx's locator reads. tsconfig is
/// JSON with comments and trailing commas.
fn root_tsconfig_paths(ctx: &Ctx) -> Result<BTreeMap<String, Vec<String>>> {
    let Some(file) = ["tsconfig.base.json", "tsconfig.json"]
        .into_iter()
        .find(|f| ctx.workspace_root.join(f).is_file())
    else {
        return Ok(BTreeMap::new());
    };
    let raw = std::fs::read_to_string(ctx.workspace_root.join(file))?;
    let json: Json = serde_json::from_str(&imports::strip_jsonc(&raw))
        .map_err(|e| eyre::eyre!("parsing {file}: {e}"))?;
    let mut out = BTreeMap::new();
    for (k, v) in json
        .pointer("/compilerOptions/paths")
        .and_then(Json::as_object)
        .into_iter()
        .flatten()
    {
        if k.matches('*').count() > 1 {
            continue;
        }
        let list = v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .map(str::to_string)
            .collect();
        out.insert(k.clone(), list);
    }
    Ok(out)
}

/// The import specifiers of one JavaScript/TypeScript file — what nx's
/// native `findImports` reports: `import … from 'x'`, `import 'x'`,
/// `export … from 'x'` and `require('x')` (static), `import('x')` (dynamic).
/// A small lexer skips comments, strings, template literals and regular
/// expressions, so text that merely looks like an import is not one.
mod imports {
    use crate::infer::DepKind;

    #[derive(Debug, PartialEq)]
    enum Tok {
        Ident(String),
        Str(String),
        Punct(char),
        Other,
    }

    fn lex(src: &str) -> Vec<Tok> {
        let chars: Vec<char> = src.chars().collect();
        let mut out = Vec::new();
        // Brace depth at which each open template substitution resumes.
        let mut templates: Vec<usize> = Vec::new();
        let mut depth = 0usize;
        let mut i = 0;
        let regex_allowed = |out: &Vec<Tok>| match out.last() {
            None => true,
            Some(Tok::Punct(c)) => !matches!(c, ')' | ']' | '}'),
            Some(Tok::Ident(w)) => matches!(
                w.as_str(),
                "return"
                    | "typeof"
                    | "instanceof"
                    | "in"
                    | "of"
                    | "new"
                    | "delete"
                    | "void"
                    | "throw"
                    | "case"
                    | "do"
                    | "else"
                    | "yield"
                    | "await"
            ),
            Some(_) => false,
        };
        while i < chars.len() {
            let c = chars[i];
            if c.is_whitespace() {
                i += 1;
            } else if c == '/' && chars.get(i + 1) == Some(&'/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            } else if c == '/' && chars.get(i + 1) == Some(&'*') {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
            } else if c == '\'' || c == '"' {
                let mut s = String::new();
                i += 1;
                while i < chars.len() && chars[i] != c && chars[i] != '\n' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    if let Some(&ch) = chars.get(i) {
                        s.push(ch);
                    }
                    i += 1;
                }
                i += 1;
                out.push(Tok::Str(s));
            } else if c == '`' || (c == '}' && templates.last() == Some(&depth)) {
                if c == '}' {
                    templates.pop();
                }
                // Template text up to the closing backtick or a `${`.
                i += 1;
                while i < chars.len() {
                    match chars[i] {
                        '\\' => i += 2,
                        '`' => {
                            i += 1;
                            break;
                        }
                        '$' if chars.get(i + 1) == Some(&'{') => {
                            i += 2;
                            templates.push(depth);
                            break;
                        }
                        _ => i += 1,
                    }
                }
                out.push(Tok::Other);
            } else if c == '/' && regex_allowed(&out) {
                let mut class = false;
                i += 1;
                while i < chars.len() && chars[i] != '\n' {
                    match chars[i] {
                        '\\' => i += 1,
                        '[' => class = true,
                        ']' => class = false,
                        '/' if !class => break,
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
                while i < chars.len() && chars[i].is_alphanumeric() {
                    i += 1;
                }
                out.push(Tok::Other);
            } else if c.is_alphabetic() || c == '_' || c == '$' {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
                {
                    i += 1;
                }
                out.push(Tok::Ident(chars[start..i].iter().collect()));
            } else if c.is_ascii_digit() {
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || chars[i] == '.' || chars[i] == '_')
                {
                    i += 1;
                }
                out.push(Tok::Other);
            } else {
                match c {
                    '{' => depth += 1,
                    '}' => depth = depth.saturating_sub(1),
                    _ => {}
                }
                out.push(Tok::Punct(c));
                i += 1;
            }
        }
        out
    }

    /// Skip an import/export clause (`x`, `* as y`, `{ a, b as c }`,
    /// `type …`, and commas between them) and return the index of what
    /// follows it.
    fn skip_clause(toks: &[Tok], mut i: usize) -> usize {
        loop {
            match toks.get(i) {
                Some(Tok::Ident(w)) if w != "from" => i += 1,
                Some(Tok::Punct('*' | ',')) => i += 1,
                Some(Tok::Punct('{')) => {
                    while i < toks.len() && toks[i] != Tok::Punct('}') {
                        i += 1;
                    }
                    i += 1;
                }
                _ => return i,
            }
        }
    }

    fn from_str(toks: &[Tok], i: usize) -> Option<&String> {
        match (toks.get(i), toks.get(i + 1)) {
            (Some(Tok::Ident(w)), Some(Tok::Str(s))) if w == "from" => Some(s),
            _ => None,
        }
    }

    pub fn find(src: &str) -> Vec<(String, DepKind)> {
        let toks = lex(src);
        let mut out = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            let Tok::Ident(word) = t else { continue };
            if i > 0 && toks[i - 1] == Tok::Punct('.') {
                continue;
            }
            match word.as_str() {
                "import" => match toks.get(i + 1) {
                    Some(Tok::Str(s)) => out.push((s.clone(), DepKind::Static)),
                    Some(Tok::Punct('(')) => {
                        if let Some(Tok::Str(s)) = toks.get(i + 2) {
                            out.push((s.clone(), DepKind::Dynamic));
                        }
                    }
                    Some(Tok::Punct('.')) => {}
                    _ => {
                        if let Some(s) = from_str(&toks, skip_clause(&toks, i + 1)) {
                            out.push((s.clone(), DepKind::Static));
                        }
                    }
                },
                "export" => {
                    let starts_clause = match toks.get(i + 1) {
                        Some(Tok::Punct('*' | '{')) => true,
                        Some(Tok::Ident(w)) if w == "type" => {
                            matches!(toks.get(i + 2), Some(Tok::Punct('*' | '{')))
                        }
                        _ => false,
                    };
                    if starts_clause && let Some(s) = from_str(&toks, skip_clause(&toks, i + 1)) {
                        out.push((s.clone(), DepKind::Static));
                    }
                }
                "require" => {
                    if let (Some(Tok::Punct('(')), Some(Tok::Str(s))) =
                        (toks.get(i + 1), toks.get(i + 2))
                    {
                        out.push((s.clone(), DepKind::Static));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// JSON-with-comments to JSON: drop comments and trailing commas.
    pub fn strip_jsonc(src: &str) -> String {
        let chars: Vec<char> = src.chars().collect();
        let mut out = String::with_capacity(src.len());
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '"' {
                out.push(c);
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        out.push(chars[i]);
                        i += 1;
                    }
                    if let Some(&ch) = chars.get(i) {
                        out.push(ch);
                    }
                    i += 1;
                }
                out.push('"');
                i += 1;
            } else if c == '/' && chars.get(i + 1) == Some(&'/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            } else if c == '/' && chars.get(i + 1) == Some(&'*') {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
            } else if c == ',' {
                let next = chars[i + 1..].iter().find(|ch| !ch.is_whitespace());
                if !matches!(next, Some('}' | ']')) {
                    out.push(c);
                }
                i += 1;
            } else {
                out.push(c);
                i += 1;
            }
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn finds_every_import_form_and_nothing_in_comments_or_strings() {
            let src = r#"
                // import x from 'commented';
                /* require('also-commented') */
                import a, { b as c } from '@ui/web-auth';
                import type { T } from "./types";
                import './side-effect';
                export * from '../lib';
                export { d } from 'pkg/sub';
                export const e = `import f from 'template' ${ g({ h: 1 }) } done`;
                const r = /from 'regex'/g;
                const i = require('cjs');
                const j = await import('lazy');
                foo.import('not-an-import');
                const k = "import l from 'string'";
            "#;
            let found: Vec<(String, DepKind)> = find(src);
            assert_eq!(
                found,
                vec![
                    ("@ui/web-auth".into(), DepKind::Static),
                    ("./types".into(), DepKind::Static),
                    ("./side-effect".into(), DepKind::Static),
                    ("../lib".into(), DepKind::Static),
                    ("pkg/sub".into(), DepKind::Static),
                    ("cjs".into(), DepKind::Static),
                    ("lazy".into(), DepKind::Dynamic),
                ]
            );
        }
    }
}

/// `semver.satisfies(version, range, { includePrerelease: true })` for the
/// range syntax package managers write: `||` alternatives, comparator sets,
/// `^`/`~`, x-ranges and hyphen ranges.
mod semver {
    type V = (u64, u64, u64, Vec<String>);

    fn parse(v: &str) -> Option<V> {
        let v = v.trim().trim_start_matches(['v', '=']);
        let v = v.split('+').next()?;
        let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p)));
        let mut it = core.split('.').map(|n| n.parse::<u64>().ok());
        let v = (
            it.next()??,
            it.next()??,
            it.next()??,
            pre.map_or_else(Vec::new, |p| p.split('.').map(str::to_string).collect()),
        );
        it.next().is_none().then_some(v)
    }

    fn cmp(a: &V, b: &V) -> std::cmp::Ordering {
        (a.0, a.1, a.2)
            .cmp(&(b.0, b.1, b.2))
            .then_with(|| match (a.3.is_empty(), b.3.is_empty()) {
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                (false, false) => {
                    for (x, y) in a.3.iter().zip(&b.3) {
                        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                            (Ok(x), Ok(y)) => x.cmp(&y),
                            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                            (Err(_), Err(_)) => x.cmp(y),
                        };
                        if o.is_ne() {
                            return o;
                        }
                    }
                    a.3.len().cmp(&b.3.len())
                }
            })
    }

    /// A partial version: missing or `x`/`*` parts are `None`.
    type Partial = (Option<u64>, Option<u64>, Option<u64>, Vec<String>);

    fn partial(p: &str) -> Option<Partial> {
        let p = p.trim().trim_start_matches(['v', '=']);
        let p = p.split('+').next()?;
        let (core, pre) = p.split_once('-').map_or((p, None), |(c, p)| (c, Some(p)));
        let mut parts = Vec::new();
        for n in core.split('.').filter(|s| !s.is_empty()) {
            parts.push(match n {
                "x" | "X" | "*" => None,
                n => Some(n.parse::<u64>().ok()?),
            });
        }
        if parts.len() > 3 {
            return None;
        }
        parts.resize(3, None);
        // Nothing after a wildcard counts.
        if let Some(pos) = parts.iter().position(Option::is_none) {
            parts[pos..].fill(None);
        }
        let pre = pre.map_or_else(Vec::new, |p| p.split('.').map(str::to_string).collect());
        Some((parts[0], parts[1], parts[2], pre))
    }

    /// Lower (inclusive) / upper (exclusive) bounds of one comparator.
    type Bound = (Option<(V, bool)>, Option<(V, bool)>);

    fn floor(p: &Partial) -> V {
        (
            p.0.unwrap_or(0),
            p.1.unwrap_or(0),
            p.2.unwrap_or(0),
            p.3.clone(),
        )
    }

    /// The first version above everything `p` covers (`None`: unbounded).
    fn ceiling(p: &Partial) -> Option<V> {
        let zero = vec!["0".to_string()];
        match (p.0, p.1, p.2) {
            (None, _, _) => None,
            (Some(m), None, _) => Some((m + 1, 0, 0, zero)),
            (Some(m), Some(n), None) => Some((m, n + 1, 0, zero)),
            (Some(_), Some(_), Some(_)) => None,
        }
    }

    fn comparator(c: &str) -> Option<Bound> {
        let (op, rest) = ["<=", ">=", "~>", "<", ">", "=", "^", "~"]
            .iter()
            .find_map(|op| c.strip_prefix(op).map(|r| (*op, r)))
            .unwrap_or(("", c));
        let p = partial(rest)?;
        let lo = floor(&p);
        let exact = p.2.is_some();
        let zero = || vec!["0".to_string()];
        Some(match op {
            "" | "=" if exact => (Some((lo.clone(), true)), Some((lo, true))),
            "" | "=" => (Some((lo, true)), ceiling(&p).map(|v| (v, false))),
            ">=" => (Some((lo, true)), None),
            ">" if exact => (Some((lo, false)), None),
            ">" => (ceiling(&p).map(|v| (v, true)), None),
            "<" => (None, Some((lo, false))),
            "<=" if exact => (None, Some((lo, true))),
            "<=" => (None, ceiling(&p).map(|v| (v, false))),
            "~" | "~>" => {
                let hi = match (p.0, p.1) {
                    (Some(m), Some(n)) => Some((m, n + 1, 0, zero())),
                    (Some(m), None) => Some((m + 1, 0, 0, zero())),
                    _ => None,
                };
                (Some((lo, true)), hi.map(|v| (v, false)))
            }
            "^" => {
                let hi = match (p.0, p.1, p.2) {
                    (None, _, _) => None,
                    (Some(0), None, _) => Some((1, 0, 0, zero())),
                    (Some(0), Some(0), None) => Some((0, 1, 0, zero())),
                    (Some(0), Some(0), Some(z)) => Some((0, 0, z + 1, zero())),
                    (Some(0), Some(n), _) => Some((0, n + 1, 0, zero())),
                    (Some(m), _, _) => Some((m + 1, 0, 0, zero())),
                };
                (Some((lo, true)), hi.map(|v| (v, false)))
            }
            _ => return None,
        })
    }

    fn within(v: &V, (lo, hi): &Bound) -> bool {
        let above = lo.as_ref().is_none_or(|(b, incl)| {
            let o = cmp(v, b);
            o.is_gt() || (*incl && o.is_eq())
        });
        let below = hi.as_ref().is_none_or(|(b, incl)| {
            let o = cmp(v, b);
            o.is_lt() || (*incl && o.is_eq())
        });
        above && below
    }

    pub fn satisfies(version: &str, range: &str) -> bool {
        let Some(v) = parse(version) else {
            return false;
        };
        range.split("||").any(|set| {
            let set = set.trim();
            if let Some((a, b)) = set.split_once(" - ") {
                let (Some(a), Some(b)) = (partial(a), partial(b)) else {
                    return false;
                };
                let hi = if b.2.is_some() {
                    Some((floor(&b), true))
                } else {
                    ceiling(&b).map(|v| (v, false))
                };
                return within(&v, &(Some((floor(&a), true)), hi));
            }
            // `>= 1.2` is one comparator: glue an operator to its version.
            let mut comparators: Vec<String> = Vec::new();
            for word in set.split_whitespace() {
                match comparators.last_mut() {
                    Some(last) if last.chars().all(|c| "<>=^~".contains(c)) => last.push_str(word),
                    _ => comparators.push(word.to_string()),
                }
            }
            comparators
                .iter()
                .all(|c| comparator(c).is_some_and(|b| within(&v, &b)))
        })
    }

    #[cfg(test)]
    mod tests {
        use super::satisfies;

        #[test]
        fn ranges_package_managers_write() {
            for (v, r, ok) in [
                ("1.2.3", "^1.0.0", true),
                ("2.0.0", "^1.0.0", false),
                ("0.2.5", "^0.2.0", true),
                ("0.3.0", "^0.2.0", false),
                ("1.2.9", "~1.2.3", true),
                ("1.3.0", "~1.2.3", false),
                ("1.4.0", ">=1.2 <2", true),
                ("3.1.0", "1.x || >=3", true),
                ("2.5.0", "1.x || >=3", false),
                ("0.0.1", "*", true),
                ("1.5.0", "1.2.3 - 1.6", true),
                ("0.1.0", "0.1.0", true),
                ("2.0.0-rc.1", "^1.0.0", false),
                ("1.3.0-rc.1", "^1.2.0", true),
            ] {
                assert_eq!(satisfies(v, r), ok, "{v} in {r}");
            }
        }
    }
}
