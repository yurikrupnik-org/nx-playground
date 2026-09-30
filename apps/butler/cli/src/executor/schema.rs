//! nx's `combineOptionsForExecutor` (nx 23 `utils/params.js`): how target
//! options, the selected configuration and CLI overrides become the options
//! object an executor receives. Planners pass their executor's `schema.json`
//! (the relevant properties, verbatim) so defaults, aliases, type coercion of
//! CLI strings and positional/unparsed smart defaults behave exactly as under
//! nx.

use eyre::{Result, bail, eyre};

use super::PlanCtx;
use crate::config::{Json, JsonMap};

/// Combine `ctx.options` (target options + configuration) with
/// `ctx.overrides`/`ctx.unparsed` under `schema`, then apply defaults and
/// validate.
pub fn combine_options(ctx: &PlanCtx<'_>, schema: &Json) -> Result<JsonMap> {
    let task = format!("{}:{}", ctx.project.name, ctx.target);
    let props = schema
        .get("properties")
        .and_then(Json::as_object)
        .ok_or_else(|| eyre!("{task}: executor schema without properties"))?;

    // CLI overrides: camelCase (only when the schema knows the camel name),
    // string coercion by schema type, aliases.
    let mut cli = JsonMap::new();
    for (k, v) in ctx.overrides {
        let camel = camel_case(k);
        let key = if props.contains_key(&camel) {
            camel
        } else {
            k.clone()
        };
        cli.insert(key, v.clone());
    }
    for (k, v) in cli.iter_mut() {
        if let Some((_, prop)) = find_property(k, props) {
            *v = coerce(Some(prop), v.take());
        }
    }
    let cli = convert_aliases(cli, props);

    let mut combined = convert_aliases(ctx.options.clone(), props);
    combined.extend(cli);
    smart_defaults(&mut combined, props, ctx);
    let definitions = schema.get("definitions").and_then(Json::as_object);
    set_defaults(&mut combined, props, definitions)?;
    validate_object(&combined, schema, definitions)
        .map_err(|e| eyre!("{task}: options do not match the executor schema: {e}"))?;
    Ok(combined)
}

/// nx `camelCase`: only when a dash appears after the first two characters.
fn camel_case(input: &str) -> String {
    match input.find('-') {
        Some(i) if i > 1 => {
            let lower = input.to_lowercase();
            let mut out = String::with_capacity(lower.len());
            let mut chars = lower.chars();
            while let Some(c) = chars.next() {
                if c == '-' {
                    match chars.next() {
                        Some(n) => out.extend(n.to_uppercase()),
                        None => out.push('-'),
                    }
                } else {
                    out.push(c);
                }
            }
            out
        }
        _ => input.to_string(),
    }
}

fn find_property<'s>(name: &str, props: &'s JsonMap) -> Option<(&'s str, &'s Json)> {
    if let Some((k, v)) = props.get_key_value(name) {
        return Some((k.as_str(), v));
    }
    props.iter().find_map(|(k, d)| {
        let alias = d.get("alias").and_then(Json::as_str) == Some(name);
        let aliases = d
            .get("aliases")
            .and_then(Json::as_array)
            .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(name)));
        (alias || aliases).then_some((k.as_str(), d))
    })
}

fn convert_aliases(opts: JsonMap, props: &JsonMap) -> JsonMap {
    let mut out = JsonMap::new();
    for (k, v) in opts {
        let name = find_property(&k, props).map_or(k, |(n, _)| n.to_string());
        out.insert(name, v);
    }
    out
}

/// nx `coerceType`: only strings are coerced.
fn coerce(prop: Option<&Json>, value: Json) -> Json {
    let Some(prop) = prop else { return value };
    let Json::String(s) = &value else {
        return value;
    };
    if let Some(alts) = prop.get("oneOf").and_then(Json::as_array) {
        for alt in alts {
            let c = coerce(Some(alt), value.clone());
            if c != value {
                return c;
            }
        }
        return value;
    }
    match prop.get("type") {
        Some(Json::Array(types)) => {
            for t in types {
                let c = coerce(Some(&serde_json::json!({ "type": t })), value.clone());
                if c != value {
                    return c;
                }
            }
            value
        }
        Some(Json::String(t)) => match t.as_str() {
            "boolean" if s.contains("true") || s.contains("false") => Json::Bool(s == "true"),
            "number" | "integer" => match js_to_number(s) {
                Some(n) => Json::Number(n),
                None => value,
            },
            "array" => {
                let items = prop.get("items").filter(|i| !i.is_array());
                Json::Array(
                    s.split(',')
                        .map(|v| coerce(items, Json::String(v.into())))
                        .collect(),
                )
            }
            _ => value,
        },
        _ => value,
    }
}

/// JS `Number(s)` when `!isNaN(+s)`.
fn js_to_number(s: &str) -> Option<serde_json::Number> {
    let t = s.trim();
    if t.is_empty() {
        return Some(0.into());
    }
    let f: f64 = t.parse().ok().filter(|f: &f64| f.is_finite())?;
    if f.fract() == 0.0 && f.abs() < 9e15 {
        Some((f as i64).into())
    } else {
        serde_json::Number::from_f64(f)
    }
}

/// nx `convertSmartDefaultsIntoNamedParams`, minus the generator-only
/// sources (`projectName` is still honored: executors can declare it).
fn smart_defaults(opts: &mut JsonMap, props: &JsonMap, ctx: &PlanCtx<'_>) {
    let argv: Vec<Json> = opts
        .get("_")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    let mut used = vec![false; argv.len()];
    for (k, v) in props {
        let Some(default) = v.get("$default") else {
            continue;
        };
        let source = default.get("$source").and_then(Json::as_str);
        match source {
            Some("argv") if !opts.contains_key(k) => {
                let index = default.get("index").and_then(Json::as_u64).unwrap_or(0) as usize;
                if let Some(value) = argv.get(index).filter(|a| is_truthy(a)) {
                    used[index] = true;
                    let value = coerce(Some(v), value.clone());
                    opts.insert(k.clone(), value);
                }
            }
            Some("unparsed") => {
                let list = ctx.unparsed.iter().cloned().map(Json::String).collect();
                opts.insert(k.clone(), Json::Array(list));
            }
            Some("projectName") if !opts.contains_key(k) => {
                opts.insert(k.clone(), Json::String(ctx.project.name.clone()));
            }
            _ => {}
        }
    }
    let left: Vec<Json> = argv
        .into_iter()
        .zip(used)
        .filter(|(_, u)| !u)
        .map(|(a, _)| a)
        .collect();
    if left.is_empty() {
        opts.remove("_");
    } else {
        opts.insert("_".into(), Json::Array(left));
    }
}

fn is_truthy(v: &Json) -> bool {
    match v {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Json::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn resolve_ref<'s>(schema: &'s Json, definitions: Option<&'s JsonMap>) -> Result<&'s Json> {
    let Some(r) = schema.get("$ref").and_then(Json::as_str) else {
        return Ok(schema);
    };
    let name = r
        .strip_prefix("#/definitions/")
        .ok_or_else(|| eyre!("$ref should start with \"#/definitions/\""))?;
    definitions
        .and_then(|d| d.get(name))
        .ok_or_else(|| eyre!("Cannot resolve {r}"))
}

fn set_defaults(opts: &mut JsonMap, props: &JsonMap, definitions: Option<&JsonMap>) -> Result<()> {
    for (name, prop) in props {
        let prop = resolve_ref(prop, definitions)?;
        let mut default = None;
        if prop.get("type").and_then(Json::as_str) == Some("array") {
            let items = prop.get("items");
            let item_props = items
                .filter(|i| i.get("type").and_then(Json::as_str) == Some("object"))
                .and_then(|i| i.get("properties"))
                .and_then(Json::as_object);
            let current_falsy = opts.get(name).is_none_or(|c| !is_truthy(c));
            match (opts.get_mut(name), item_props) {
                (Some(Json::Array(values)), Some(item_props)) => {
                    for v in values.iter_mut().filter_map(Json::as_object_mut) {
                        set_defaults(v, item_props, definitions)?;
                    }
                }
                _ if current_falsy => {
                    default = prop.get("default").filter(|d| is_truthy(d)).cloned();
                }
                _ => {}
            }
        } else {
            if !opts.contains_key(name) {
                default = prop.get("default").cloned();
            }
            if prop.get("type").and_then(Json::as_str) == Some("object")
                && let Some(Json::Object(inner)) = opts.get_mut(name)
            {
                let inner_props = prop
                    .get("properties")
                    .and_then(Json::as_object)
                    .cloned()
                    .unwrap_or_default();
                set_defaults(inner, &inner_props, definitions)?;
            }
        }
        // nx silently skips a default that fails its own schema.
        if let Some(d) = default
            && validate_property(name, &d, Some(prop), definitions).is_ok()
        {
            opts.insert(name.clone(), d);
        }
    }
    Ok(())
}

fn validate_object(opts: &JsonMap, schema: &Json, definitions: Option<&JsonMap>) -> Result<()> {
    if let Some(any) = schema.get("anyOf").and_then(Json::as_array) {
        let errors: Vec<String> = any
            .iter()
            .filter_map(|s| validate_object(opts, s, definitions).err())
            .map(|e| e.to_string())
            .collect();
        if errors.len() == any.len() {
            bail!("options did not match any of: {}", errors.join("; "));
        }
    }
    if let Some(one) = schema.get("oneOf").and_then(Json::as_array) {
        let matches = one
            .iter()
            .filter(|s| validate_object(opts, s, definitions).is_ok())
            .count();
        if matches != 1 {
            bail!("options must match exactly one `oneOf` alternative (matched {matches})");
        }
    }
    for req in schema
        .get("required")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(r) = req.as_str()
            && !opts.contains_key(r)
        {
            bail!("Required property '{r}' is missing");
        }
    }
    let props = schema.get("properties").and_then(Json::as_object);
    match schema.get("additionalProperties") {
        None | Some(Json::Bool(true)) => {}
        Some(extra) => {
            for (p, v) in opts {
                if props.is_some_and(|ps| ps.contains_key(p)) {
                    continue;
                }
                if p == "_" {
                    bail!("Schema does not support positional arguments. Argument '{v}' found");
                }
                match extra {
                    Json::Bool(false) => bail!("'{p}' is not found in schema"),
                    Json::Object(_) => validate_property(p, v, Some(extra), definitions)?,
                    _ => {}
                }
            }
        }
    }
    for (p, v) in opts {
        validate_property(p, v, props.and_then(|ps| ps.get(p)), definitions)?;
    }
    Ok(())
}

fn validate_property(
    name: &str,
    value: &Json,
    schema: Option<&Json>,
    definitions: Option<&JsonMap>,
) -> Result<()> {
    let Some(schema) = schema else { return Ok(()) };
    let schema = resolve_ref(schema, definitions)?;
    let with_type = |r: &Json| -> Json {
        let mut rule = JsonMap::new();
        if let Some(t) = schema.get("type") {
            rule.insert("type".into(), t.clone());
        }
        if let Some(o) = r.as_object() {
            rule.extend(o.clone());
        }
        Json::Object(rule)
    };
    let invalid = || eyre!("Property '{name}' does not match the schema");
    if let Some(one) = schema.get("oneOf").and_then(Json::as_array) {
        let passes = one
            .iter()
            .filter(|r| validate_property(name, value, Some(&with_type(r)), definitions).is_ok())
            .count();
        return if passes == 1 { Ok(()) } else { Err(invalid()) };
    }
    if let Some(any) = schema.get("anyOf").and_then(Json::as_array) {
        let passes = any
            .iter()
            .any(|r| validate_property(name, value, Some(&with_type(r)), definitions).is_ok());
        return if passes { Ok(()) } else { Err(invalid()) };
    }
    if let Some(all) = schema.get("allOf").and_then(Json::as_array) {
        let passes = all
            .iter()
            .all(|r| validate_property(name, value, Some(&with_type(r)), definitions).is_ok());
        return if passes { Ok(()) } else { Err(invalid()) };
    }
    let ty = schema.get("type");
    match value {
        Json::Array(items) => {
            if ty.and_then(Json::as_str) != Some("array") {
                return Err(invalid());
            }
            match schema.get("items") {
                Some(Json::Array(tuple)) => {
                    for (i, item) in items.iter().enumerate() {
                        match tuple.get(i) {
                            Some(s) => validate_property(name, item, Some(s), definitions)?,
                            None if schema.get("additionalItems") == Some(&Json::Bool(false)) => {
                                return Err(invalid());
                            }
                            None => {
                                if let Some(extra) =
                                    schema.get("additionalItems").filter(|e| e.is_object())
                                {
                                    validate_property(name, item, Some(extra), definitions)?;
                                }
                            }
                        }
                    }
                }
                items_schema => {
                    for item in items {
                        validate_property(name, item, items_schema, definitions)?;
                    }
                }
            }
        }
        Json::Null => {
            let ok = match ty {
                Some(Json::Array(ts)) => ts.iter().any(|t| t.as_str() == Some("null")),
                other => other.and_then(Json::as_str) == Some("null"),
            };
            if !ok {
                return Err(invalid());
            }
        }
        Json::Object(inner) => {
            if ty.and_then(Json::as_str) != Some("object") {
                return Err(invalid());
            }
            validate_object(inner, schema, definitions)?;
        }
        primitive => {
            if let Some(c) = schema.get("const")
                && c != primitive
            {
                bail!(
                    "Property '{name}' does not match the schema. '{primitive}' should be '{c}'."
                );
            }
            let js_type = match primitive {
                Json::Bool(_) => "boolean",
                Json::Number(_) => "number",
                _ => "string",
            };
            let type_ok = |t: &str| (if t == "integer" { "number" } else { t }) == js_type;
            match ty {
                Some(Json::Array(ts)) if !ts.iter().filter_map(Json::as_str).any(type_ok) => {
                    bail!(
                        "Property '{name}' does not match the schema. '{primitive}' should be a '{}'.",
                        Json::Array(ts.clone())
                    );
                }
                Some(Json::String(t)) if !type_ok(t) => {
                    bail!(
                        "Property '{name}' does not match the schema. '{primitive}' should be a '{t}'."
                    );
                }
                _ => {}
            }
            if let Some(allowed) = schema.get("enum").and_then(Json::as_array)
                && !allowed.contains(primitive)
            {
                bail!(
                    "Property '{name}' does not match the schema. '{primitive}' should be one of {}.",
                    Json::Array(allowed.clone())
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_only_past_the_second_character() {
        assert_eq!(camel_case("exit-zero"), "exitZero");
        assert_eq!(camel_case("a-b"), "a-b");
        assert_eq!(camel_case("build-args"), "buildArgs");
    }

    #[test]
    fn cli_strings_coerce_by_schema_type() {
        let t = |ty: &str| serde_json::json!({ "type": ty });
        assert_eq!(
            coerce(Some(&t("boolean")), "false".into()),
            Json::Bool(false)
        );
        assert_eq!(coerce(Some(&t("number")), "3".into()), serde_json::json!(3));
        assert_eq!(
            coerce(
                Some(&serde_json::json!({"type": "array", "items": {"type": "string"}})),
                "a,b".into()
            ),
            serde_json::json!(["a", "b"])
        );
        assert_eq!(
            coerce(Some(&t("string")), "true".into()),
            Json::String("true".into())
        );
    }

    #[test]
    fn validation_rejects_unknown_and_mistyped() {
        let schema = serde_json::json!({
            "properties": {"a": {"type": "boolean"}, "b": {"type": ["string", "array"]}},
            "additionalProperties": false
        });
        let ok: JsonMap = serde_json::from_value(serde_json::json!({"a": true, "b": "x"})).unwrap();
        assert!(validate_object(&ok, &schema, None).is_ok());
        let unknown: JsonMap = serde_json::from_value(serde_json::json!({"c": 1})).unwrap();
        assert!(validate_object(&unknown, &schema, None).is_err());
        let mistyped: JsonMap = serde_json::from_value(serde_json::json!({"a": "yes"})).unwrap();
        assert!(validate_object(&mistyped, &schema, None).is_err());
    }
}
