//! The slice of `yargs-parser` (v21, what nx 23 bundles) that decides how CLI
//! overrides reach executors. nx parses the args it does not own twice — once
//! into typed task overrides (`createOverrides`) and once inside run-commands
//! to decide which ones to forward — and the two parses use different
//! settings, so both are reproduced here rather than approximated.

use crate::config::{Json, JsonMap};

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub parse_numbers: bool,
    pub parse_positional_numbers: bool,
    pub dot_notation: bool,
    pub camel_case_expansion: bool,
}

/// nx `createOverrides`: dot-notation on, camel-case expansion off.
pub const OVERRIDES: Config = Config {
    parse_numbers: true,
    parse_positional_numbers: true,
    dot_notation: true,
    camel_case_expansion: false,
};

/// run-commands' parse of `__unparsed__`: nothing coerced, keys kept verbatim.
pub const RUN_COMMANDS_UNPARSED: Config = Config {
    parse_numbers: false,
    parse_positional_numbers: false,
    dot_notation: false,
    camel_case_expansion: false,
};

/// run-commands' parse of its own `args` option.
pub const RUN_COMMANDS_ARGS: Config = Config {
    parse_numbers: true,
    parse_positional_numbers: true,
    dot_notation: true,
    camel_case_expansion: true,
};

/// Parse `args` (no declared options, as nx calls it). The result always has
/// `_` (positionals).
pub fn parse(args: &[String], cfg: Config) -> JsonMap {
    let mut out = JsonMap::new();
    let mut positional: Vec<Json> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let next = args.get(i + 1).map(String::as_str);
        if arg == "--" {
            for rest in &args[i + 1..] {
                positional.push(coerce(rest, cfg, true));
            }
            break;
        }
        if let Some(body) = arg.strip_prefix("--")
            && let Some((key, value)) = body.split_once('=')
            && !key.is_empty()
        {
            set_arg(&mut out, key, Json::String(value.into()), cfg);
        } else if let Some(key) = arg.strip_prefix("--no-")
            && !key.is_empty()
        {
            set_arg(&mut out, key, Json::Bool(false), cfg);
        } else if let Some(key) = arg.strip_prefix("--")
            && !key.is_empty()
        {
            match next {
                Some(n) if !n.starts_with('-') || is_negative(n) => {
                    set_arg(&mut out, key, Json::String(n.into()), cfg);
                    i += 1;
                }
                _ => set_arg(&mut out, key, Json::Bool(true), cfg),
            }
        } else if arg.len() > 1
            && arg.starts_with('-')
            && !arg.starts_with("--")
            && !is_negative(arg)
        {
            short_group(arg, next, &mut out, &mut i, cfg);
        } else {
            positional.push(coerce(arg, cfg, true));
        }
        i += 1;
    }
    out.insert("_".into(), Json::Array(positional));
    out
}

/// `-abc`, `-n5`, `-k=v`, `-k value`.
fn short_group(arg: &str, next: Option<&str>, out: &mut JsonMap, i: &mut usize, cfg: Config) {
    let chars: Vec<char> = arg[1..].chars().collect();
    let letters = &chars[..chars.len() - 1];
    let mut broken = false;
    for (j, letter) in letters.iter().enumerate() {
        let rest: String = chars[j + 1..].iter().collect();
        let key = letter.to_string();
        if chars.get(j + 1) == Some(&'=') {
            let value: String = chars[j + 2..].iter().collect();
            set_arg(out, &key, Json::String(value), cfg);
            broken = true;
            break;
        }
        if rest == "-" {
            set_arg(out, &key, Json::String(rest), cfg);
            continue;
        }
        if letter.is_ascii_alphabetic() && is_short_number(&rest) {
            set_arg(out, &key, Json::String(rest), cfg);
            broken = true;
            break;
        }
        if chars
            .get(j + 1)
            .is_some_and(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
        {
            set_arg(out, &key, Json::String(rest), cfg);
            broken = true;
            break;
        }
        set_arg(out, &key, Json::Bool(true), cfg);
    }
    let last = chars[chars.len() - 1];
    if !broken && last != '-' {
        let key = last.to_string();
        match next {
            Some(n)
                if !(n.len() > 1 && n.starts_with('-') && !n[1..].starts_with('-'))
                    && !(n.len() > 2 && n.starts_with("--") && !n[2..].starts_with('-'))
                    || is_negative(n) =>
            {
                set_arg(out, &key, Json::String(n.into()), cfg);
                *i += 1;
            }
            _ => set_arg(out, &key, Json::Bool(true), cfg),
        }
    }
}

fn is_negative(s: &str) -> bool {
    let digits = s.trim_start_matches('-');
    digits.len() < s.len() && digits.starts_with(|c: char| c.is_ascii_digit())
}

/// `/^-?\d+(\.\d*)?(e-?\d+)?$/`
fn is_short_number(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let (mantissa, exp) = match s.split_once('e') {
        Some((m, e)) => (m, Some(e.strip_prefix('-').unwrap_or(e))),
        None => (s, None),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    !int.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit())
        && frac.bytes().all(|b| b.is_ascii_digit())
        && exp.is_none_or(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit()))
}

fn set_arg(out: &mut JsonMap, key: &str, value: Json, cfg: Config) {
    let value = match value {
        Json::String(s) => {
            let unquoted = strip_matching_quotes(&s);
            coerce(unquoted, cfg, false)
        }
        other => other,
    };
    if cfg.camel_case_expansion && key.contains('-') && !key.contains('.') {
        set_key(out, &[camel_case(key)], value.clone());
    }
    let path: Vec<String> = if cfg.dot_notation {
        key.split('.').map(str::to_string).collect()
    } else {
        vec![key.to_string()]
    };
    set_key(out, &path, value);
}

fn strip_matching_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'\'' || bytes[0] == b'"')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Nested assignment; a repeated key becomes an array
/// (`duplicate-arguments-array`).
fn set_key(out: &mut JsonMap, path: &[String], value: Json) {
    let (last, parents) = path.split_last().expect("non-empty key path");
    let mut cur = out;
    for p in parents {
        let slot = cur
            .entry(p.clone())
            .or_insert_with(|| Json::Object(JsonMap::new()));
        if !slot.is_object() {
            *slot = Json::Object(JsonMap::new());
        }
        cur = slot.as_object_mut().expect("just ensured object");
    }
    match cur.get_mut(last) {
        None => {
            cur.insert(last.clone(), value);
        }
        Some(Json::Array(list)) => list.push(value),
        Some(existing) => {
            let prev = existing.take();
            *existing = Json::Array(vec![prev, value]);
        }
    }
}

fn camel_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut upper = false;
    for c in key.chars() {
        if c == '-' || c == '_' {
            upper = !out.is_empty();
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// yargs-parser `maybeCoerceNumber`.
fn coerce(s: &str, cfg: Config, positional: bool) -> Json {
    let enabled = if positional {
        cfg.parse_positional_numbers
    } else {
        cfg.parse_numbers
    };
    if enabled && let Some(n) = js_number(s) {
        return Json::Number(n);
    }
    Json::String(s.to_string())
}

/// `isNumber` + `Number.isSafeInteger(Math.floor(parseFloat(x)))`.
fn js_number(s: &str) -> Option<serde_json::Number> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).ok().map(Into::into);
    }
    if s.len() > 1 && s.starts_with('0') && !s[1..].starts_with('.') {
        return None;
    }
    let body = s.strip_prefix('-').unwrap_or(s);
    let (mantissa, exp) = match body.split_once(['e', 'E']) {
        Some((m, e)) if body[m.len()..].starts_with('e') => (m, Some(e)),
        Some(_) => return None,
        None => (body, None),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
    let mantissa_ok = if mantissa.contains('.') {
        (!int.is_empty() || !frac.is_empty()) && digits(int) && digits(frac)
    } else {
        !int.is_empty() && digits(int)
    };
    let exp_ok = exp.is_none_or(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        !e.is_empty() && digits(e)
    });
    if !mantissa_ok || !exp_ok {
        return None;
    }
    let f: f64 = s.parse().ok()?;
    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
    if f.floor().abs() > MAX_SAFE {
        return None;
    }
    if f.fract() == 0.0 && !mantissa.contains('.') && exp.is_none() {
        // Keep integers integral so they print like JS numbers.
        return Some((f as i64).into());
    }
    serde_json::Number::from_f64(f)
}

/// JavaScript `String(value)` for option values — how nx splices options
/// into command lines.
pub fn js_string(v: &Json) -> String {
    match v {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 1e21 => format!("{f:.0}"),
            _ => n.to_string(),
        },
        Json::String(s) => s.clone(),
        Json::Array(items) => items
            .iter()
            .map(|i| match i {
                Json::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Json::Object(_) => "[object Object]".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(args: &[&str], cfg: Config) -> Json {
        let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        Json::Object(parse(&args, cfg))
    }

    #[test]
    fn flags_values_negation_and_positionals() {
        assert_eq!(
            p(
                &["--check", "--fix=false", "--no-color", "src", "--jobs", "4"],
                OVERRIDES
            ),
            json!({"check": true, "fix": "false", "color": false, "jobs": 4, "_": ["src"]})
        );
    }

    #[test]
    fn a_flag_followed_by_a_flag_is_boolean() {
        assert_eq!(
            p(&["--watch", "--port=3000"], OVERRIDES),
            json!({"watch": true, "port": 3000, "_": []})
        );
    }

    #[test]
    fn dot_notation_and_duplicates() {
        assert_eq!(
            p(&["--env.FOO=bar", "--x", "a", "--x", "b"], OVERRIDES),
            json!({"env": {"FOO": "bar"}, "x": ["a", "b"], "_": []})
        );
        // run-commands keeps keys verbatim and numbers as strings.
        assert_eq!(
            p(&["--env.FOO=bar", "--n=1", "7"], RUN_COMMANDS_UNPARSED),
            json!({"env.FOO": "bar", "n": "1", "_": ["7"]})
        );
    }

    #[test]
    fn short_groups_and_double_dash() {
        assert_eq!(
            p(&["-abc", "-n5", "--", "--literal"], OVERRIDES),
            json!({"a": true, "b": true, "c": true, "n": 5, "_": ["--literal"]})
        );
    }

    #[test]
    fn numbers_follow_js_rules() {
        assert_eq!(
            p(&["--a=0123", "--b=1.5", "--c=0x10"], OVERRIDES)["a"],
            json!("0123")
        );
        assert_eq!(p(&["--b=1.5"], OVERRIDES)["b"], json!(1.5));
        assert_eq!(p(&["--c=0x10"], OVERRIDES)["c"], json!(16));
        assert_eq!(js_string(&json!(1.0)), "1");
        assert_eq!(js_string(&json!(["a", 2, true])), "a,2,true");
    }
}
