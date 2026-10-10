//! Post-processing and invariant checks for the committed OpenAPI documents
//! under `docs/openapi/` (feature `openapi`).
//!
//! Those documents are generated artifacts (`export_openapi_*` tests) and the
//! `x` CLI derives its entire command tree from them and nothing else: an
//! operation's tag is the resource name, its `operationId` is the verb, and a
//! `{placeholder}` in the path key is a positional argument that must be
//! declared as a typed, required path parameter. That makes the checks below
//! load-bearing, so they live here once instead of being re-implemented (and
//! drifting) in every API crate.
//!
//! One invariant is deliberately absent: "error statuses reference a response
//! body schema where the handler actually returns one" cannot be decided from
//! the document — whether a handler returns `ErrorResponse` or a bare string is
//! only visible in the handler. That one stays a review item.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

/// Operation keys an OpenAPI 3.1 path item may carry, per the specification.
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Rewrite a freshly built document into the shape committed under `docs/openapi/`.
///
/// Both rewrites exist because `utoipa::openapi::OpenApi::nest` composes paths
/// with a plain string concatenation while axum's nesting does not:
///
/// * A nested document's root operation contributes `""` (from `path = ""`) or
///   `"/"` (from `path = "/"`). axum's `path_for_nested_route` maps an inner `/`
///   onto the prefix *exactly*, and axum 0.8 does not redirect `/api/todos/` to
///   `/api/todos`, so a trailing slash would name a route the server never
///   serves. A trailing slash is therefore stripped from every key longer than
///   one character, and an empty key becomes `"/"` — correct for a standalone
///   document, whose `servers[0].url` *is* the mount path.
/// * `nest` also merges the nested document's `servers` into this one. Those
///   URLs describe where that document is mounted standalone (`/api/tasks`),
///   not an alternative origin for this document, so only the first survives.
pub fn normalize(doc: &mut Value) {
    if let Some(servers) = doc.get_mut("servers").and_then(Value::as_array_mut) {
        servers.truncate(1);
    }

    let Some(paths) = doc.get_mut("paths").and_then(Value::as_object_mut) else {
        return;
    };

    let rewrites = paths
        .keys()
        .filter_map(|key| {
            let rewritten = if key.is_empty() {
                "/".to_owned()
            } else if key.len() > 1 && key.ends_with('/') {
                key.trim_end_matches('/').to_owned()
            } else {
                return None;
            };
            Some((key.clone(), rewritten))
        })
        .collect::<Vec<_>>();

    for (from, to) in rewrites {
        let item = paths.remove(&from).expect("key was just listed");
        match (paths.get_mut(&to), item) {
            // `/x` and `/x/` both present: fold the operations together rather
            // than silently dropping one of them.
            (Some(Value::Object(existing)), Value::Object(item)) => existing.extend(item),
            (_, item) => {
                paths.insert(to, item);
            }
        }
    }
}

/// [`normalize`] as a value transform, for callers that only want to inspect.
#[must_use]
pub fn normalized(mut doc: Value) -> Value {
    normalize(&mut doc);
    doc
}

/// Panic unless `doc` satisfies every invariant the `x` CLI relies on.
///
/// # Panics
///
/// With a message naming the offending operation, so a handler added without
/// `params(...)`, without a tag, or with a colliding `operationId` fails the
/// owning crate's export test instead of breaking the CLI at runtime.
pub fn assert_invariants(doc: &Value) {
    assert_eq!(
        doc.get("openapi").and_then(Value::as_str),
        Some("3.1.0"),
        "committed documents are OpenAPI 3.1.0"
    );

    let server = doc
        .get("servers")
        .and_then(Value::as_array)
        .and_then(|servers| servers.first())
        .and_then(|server| server.get("url"))
        .and_then(Value::as_str)
        .expect("servers[0].url is the mount path relative to the process origin");
    assert!(
        server.starts_with('/'),
        "servers[0].url must be a mount path relative to the process origin, got {server:?}"
    );

    let paths = doc
        .get("paths")
        .and_then(Value::as_object)
        .expect("paths object");
    assert!(!paths.is_empty(), "document declares no operations");

    // operationId -> the "GET /todos" it was first seen on.
    let mut seen: HashMap<&str, String> = HashMap::new();

    for (path, item) in paths {
        assert!(
            path.starts_with('/'),
            "path key {path:?} must start with '/'"
        );
        let item = item
            .as_object()
            .unwrap_or_else(|| panic!("path item {path:?} is not an object"));
        // Parameters may be declared once for the whole path item or per
        // operation; the spec takes the union, so accept both.
        let shared = item.get("parameters").and_then(Value::as_array);

        for (method, operation) in item
            .iter()
            .filter(|(key, _)| METHODS.contains(&key.as_str()))
        {
            let route = format!("{} {path}", method.to_uppercase());
            let operation = operation
                .as_object()
                .unwrap_or_else(|| panic!("{route} is not an operation object"));

            let id = operation
                .get("operationId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            assert!(
                !id.is_empty(),
                "{route} has no operationId; the CLI derives its verb from it"
            );
            if let Some(previous) = seen.insert(id, route.clone()) {
                panic!("operationId {id:?} is not unique: {previous} and {route}");
            }

            let tagged = operation
                .get("tags")
                .and_then(Value::as_array)
                .is_some_and(|tags| {
                    tags.iter()
                        .any(|tag| tag.as_str().is_some_and(|tag| !tag.is_empty()))
                });
            assert!(
                tagged,
                "{route} ({id}) has no tag; the tag is the CLI resource name"
            );

            for placeholder in placeholders(path) {
                let declared = shared
                    .into_iter()
                    .chain(operation.get("parameters").and_then(Value::as_array))
                    .flatten()
                    .find(|parameter| {
                        parameter.get("name").and_then(Value::as_str) == Some(placeholder)
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "{route} ({id}) templates {{{placeholder}}} but declares no \
                             {placeholder:?} parameter; add params((\"{placeholder}\" = T, Path, ..))"
                        )
                    });
                assert_eq!(
                    declared.get("in").and_then(Value::as_str),
                    Some("path"),
                    "{route} ({id}) declares {placeholder:?} outside the path"
                );
                assert_eq!(
                    declared.get("required").and_then(Value::as_bool),
                    Some(true),
                    "{route} ({id}) declares path parameter {placeholder:?} as optional"
                );
                assert!(
                    declared.get("schema").is_some(),
                    "{route} ({id}) declares path parameter {placeholder:?} without a schema"
                );
            }
        }
    }
}

/// Normalize, validate, and write the committed document (pretty JSON, trailing
/// newline). The document is never written unless it is valid, so a broken spec
/// cannot reach `docs/openapi/` and the drift gate.
///
/// # Panics
///
/// If the document violates [`assert_invariants`] or cannot be written.
pub fn export(doc: Value, path: impl AsRef<Path>) {
    let doc = normalized(doc);
    assert_invariants(&doc);

    let path = path.as_ref();
    let json = serde_json::to_string_pretty(&doc).expect("serialize openapi document");
    std::fs::create_dir_all(path.parent().expect("document path has a parent"))
        .expect("create docs/openapi");
    std::fs::write(path, json + "\n").unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// The path-template placeholders of `path`, without their braces.
fn placeholders(path: &str) -> impl Iterator<Item = &str> {
    path.split('/')
        .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// A minimal valid document: one tagged operation with a declared,
    /// required, typed path parameter.
    fn document() -> serde_json::Value {
        json!({
            "openapi": "3.1.0",
            "servers": [{ "url": "/api" }],
            "paths": {
                "/todos/{id}": {
                    "get": {
                        "operationId": "get_todo",
                        "tags": ["todos"],
                        "parameters": [
                            { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }
                        ]
                    }
                }
            }
        })
    }

    #[test]
    fn accepts_a_conforming_document() {
        super::assert_invariants(&document());
    }

    #[test]
    #[should_panic(expected = "declares no \"id\" parameter")]
    fn rejects_a_templated_path_without_a_path_parameter() {
        let mut doc = document();
        doc["paths"]["/todos/{id}"]["get"]
            .as_object_mut()
            .unwrap()
            .remove("parameters");
        super::assert_invariants(&doc);
    }

    #[test]
    #[should_panic(expected = "has no tag")]
    fn rejects_an_untagged_operation() {
        let mut doc = document();
        doc["paths"]["/todos/{id}"]["get"]["tags"] = json!([]);
        super::assert_invariants(&doc);
    }

    #[test]
    #[should_panic(expected = "is not unique")]
    fn rejects_duplicate_operation_ids() {
        let mut doc = document();
        let operation = doc["paths"]["/todos/{id}"]["get"].clone();
        doc["paths"]["/todos/{id}"]["put"] = operation;
        super::assert_invariants(&doc);
    }

    #[test]
    #[should_panic(expected = "must start with '/'")]
    fn rejects_a_path_key_without_a_leading_slash() {
        let mut doc = document();
        let paths = doc["paths"].as_object_mut().unwrap();
        let item = paths.remove("/todos/{id}").unwrap();
        paths.insert("todos/{id}".to_owned(), item);
        super::assert_invariants(&doc);
    }

    /// `nest` concatenates, axum does not: an inner `/` route lands on the
    /// prefix itself, and an empty key means "the mount path".
    #[test]
    fn normalize_rewrites_nested_root_keys_and_drops_merged_servers() {
        let doc = super::normalized(json!({
            "servers": [{ "url": "/api" }, { "url": "/api/tasks" }],
            "paths": {
                "/todos/": { "get": { "operationId": "list_todos" } },
                "": { "get": { "operationId": "list_tasks" } },
                "/todos/{id}": { "get": { "operationId": "get_todo" } }
            }
        }));

        let keys = doc["paths"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            keys,
            ["/", "/todos", "/todos/{id}"].into_iter().collect(),
            "nested root keys are rewritten, other keys untouched"
        );
        assert_eq!(doc["servers"].as_array().unwrap().len(), 1);
    }

    /// Folding must not lose operations when both spellings are present.
    #[test]
    fn normalize_folds_a_trailing_slash_onto_an_existing_key() {
        let doc = super::normalized(json!({
            "paths": {
                "/todos": { "get": { "operationId": "list_todos" } },
                "/todos/": { "post": { "operationId": "create_todo" } }
            }
        }));

        assert_eq!(doc["paths"]["/todos"]["get"]["operationId"], "list_todos");
        assert_eq!(doc["paths"]["/todos"]["post"]["operationId"], "create_todo");
    }
}
