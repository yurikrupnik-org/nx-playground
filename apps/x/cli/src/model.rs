//! The command model: how an OpenAPI operation becomes `x <verb> <resource>`.
//!
//! There is exactly one mapping rule and it is derived from two fields the
//! documents guarantee:
//!
//! * `operationId` is a snake_case Rust handler name, so its first segment is
//!   the **verb** (`list_todos` -> `list`, `complete_todo` -> `complete`).
//! * `tags[0]` is the **resource** (`todos`, `tasks`, `org`, ...). The tag is
//!   used rather than the operationId's tail because the tail is singular
//!   (`get_todo`) while the route and the tag are plural (`/todos`, `todos`),
//!   and a CLI that says `x get todo` for one verb and `x get todos` for
//!   another is a CLI nobody can remember.
//!
//! `list` is then folded into `get`: `x get todos` lists, `x get todos <id>`
//! fetches one. Both operations stay addressable; which one runs is decided by
//! how many positional arguments were supplied, the same way `kubectl get` does
//! it. That fold is the only place the mapping is not mechanical, and it exists
//! because it is the shape people actually type.

use std::collections::BTreeMap;

use crate::spec::{Credential, Doc, RawParam, Scalar, Schema};

/// One API process: a document plus where to reach it.
#[derive(Debug, Clone)]
pub struct Api {
    /// Registry key and `--api` value (`todo`, `zerg`, `terran`, `remote`).
    pub key: String,
    pub origin: String,
    pub doc: Doc,
}

impl Api {
    /// Absolute URL for an operation path: origin + document mount + path.
    pub fn url(&self, path: &str) -> String {
        let mount = self.doc.mount();
        // A document whose `servers[0].url` is absolute carries its own origin.
        if mount.starts_with("http://") || mount.starts_with("https://") {
            return format!("{}{}", mount.trim_end_matches('/'), path);
        }
        format!(
            "{}{}{}",
            self.origin.trim_end_matches('/'),
            mount.trim_end_matches('/'),
            path
        )
    }
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub required: bool,
    pub scalar: Scalar,
    pub choices: Option<Vec<String>>,
    pub help: String,
}

/// A body field promoted to a command-line flag.
#[derive(Debug, Clone)]
pub struct BodyField {
    pub name: String,
    pub required: bool,
    pub scalar: Scalar,
    pub choices: Option<Vec<String>>,
    pub help: String,
}

#[derive(Debug, Clone)]
pub struct Op {
    /// Index into the registry's API list.
    pub api: usize,
    pub operation_id: String,
    pub method: String,
    /// Path template as it appears in the document, e.g. `/todos/{id}`.
    pub path: String,
    pub verb: String,
    pub resource: String,
    pub summary: String,
    pub path_params: Vec<Param>,
    pub query_params: Vec<Param>,
    pub body_fields: Vec<BodyField>,
    /// True when the operation declares a request body that is not a flat
    /// object — the only way to supply it is `--body`.
    pub opaque_body: bool,
    pub has_body: bool,
    /// The credential the document declares for this operation, if any.
    /// Empty means the document says the route is public — which is a claim
    /// about the document, not proof, so a 401 is still reported with the
    /// flags that could fix it.
    pub credentials: Vec<Credential>,
}

impl Op {
    pub fn arity(&self) -> usize {
        self.path_params.len()
    }
}

/// Every operation across every registered API, indexed for lookup.
#[derive(Debug, Default)]
pub struct Registry {
    pub apis: Vec<Api>,
    pub ops: Vec<Op>,
}

/// The verb is the operationId with its trailing resource noun removed.
///
/// Splitting at the first `_` is the obvious rule and it is wrong: this
/// workspace's handler names are `<action>_<noun>` where the action may be
/// several words — `soft_delete_cloud_resource` would yield `soft`, and
/// `password_login` would yield `password`. Stripping the noun instead gives
/// `soft-delete` and leaves `password_login` whole, which is odd-looking but
/// unambiguous. That is the correct failure mode: the CLI does not invent a
/// nicer name than the one the server published. A handler that wants a
/// better command renames its own `operationId`.
///
/// `list` then folds into `get`, so arity alone decides collection-vs-item.
fn derive_verb(operation_id: &str, resource: &str) -> String {
    let noun = resource.replace('-', "_");
    let singular = singularize(&noun);

    let stem = [format!("_{noun}"), format!("_{singular}")]
        .iter()
        .find_map(|suffix| operation_id.strip_suffix(suffix.as_str()))
        .unwrap_or(operation_id);

    let verb = if stem == "list" { "get" } else { stem };
    verb.replace('_', "-")
}

/// Enough singularization to strip a noun that this workspace's handler names
/// actually use (`todos`/`todo`, `cloud_resources`/`cloud_resource`). It is
/// never used to *build* a name, only to test a suffix, so a wrong answer
/// degrades to "no strip" rather than to a wrong command.
fn singularize(noun: &str) -> String {
    match noun.strip_suffix("ies") {
        Some(stem) => format!("{stem}y"),
        None => noun.strip_suffix('s').unwrap_or(noun).to_owned(),
    }
}

/// Tags are not written by one hand: `domain_todo` spells its tag `todos`,
/// while `libs/core/proc_macros/sea_orm_resource` derives Title Case from the
/// table name (`Projects`, `Cloud Resources`) and pins that in its own unit
/// tests. Normalizing here rather than changing the macro keeps the server
/// contract untouched and still gives one shell-friendly spelling:
/// `x get cloud-resources`, never `x get "Cloud Resources"`.
fn normalize_resource(tag: &str) -> String {
    tag.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn placeholders(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        out.push(rest[start + 1..start + end].to_owned());
        rest = &rest[start + end + 1..];
    }
    out
}

fn param(doc: &Doc, raw: &RawParam, fallback_help: &str) -> Param {
    Param {
        name: raw.name.clone(),
        required: raw.required,
        scalar: doc.scalar(&raw.schema),
        choices: doc.choices(&raw.schema),
        help: raw
            .description
            .clone()
            .or_else(|| raw.schema.description.clone())
            .unwrap_or_else(|| fallback_help.to_owned()),
    }
}

fn body_fields(doc: &Doc, schema: &Schema) -> (Vec<BodyField>, bool) {
    let resolved = doc.resolve(schema);
    if resolved.properties.is_empty() {
        return (Vec::new(), true);
    }
    let fields = resolved
        .properties
        .iter()
        .map(|(name, prop)| BodyField {
            name: name.clone(),
            required: resolved.required.iter().any(|r| r == name),
            scalar: doc.scalar(prop),
            choices: doc.choices(prop),
            help: doc
                .resolve(prop)
                .description
                .clone()
                .or_else(|| prop.description.clone())
                .unwrap_or_else(|| format!("{name} field of the request body")),
        })
        .collect();
    (fields, false)
}

impl Registry {
    pub fn build(apis: Vec<Api>) -> Self {
        let mut ops = Vec::new();

        for (index, api) in apis.iter().enumerate() {
            let doc = &api.doc;
            for (path, method, raw, shared) in doc.operations() {
                let Some(operation_id) = raw.operation_id.clone() else {
                    // Without an operationId there is no verb to derive. The
                    // invariant tests make this unreachable for our own
                    // documents; a `--spec` document may still hit it.
                    continue;
                };
                let resource = normalize_resource(
                    &raw.tags
                        .first()
                        .cloned()
                        .unwrap_or_else(|| path.trim_matches('/').replace('/', "-")),
                );
                let verb = derive_verb(&operation_id, &resource);

                let declared: Vec<&RawParam> = shared.iter().chain(raw.parameters.iter()).collect();
                let names = placeholders(&path);
                let path_params = names
                    .iter()
                    .map(|name| {
                        match declared
                            .iter()
                            .find(|p| p.location == "path" && &p.name == name)
                        {
                            Some(raw) => param(doc, raw, name),
                            // Guarded by the invariant test; kept as a typed
                            // fallback so a third-party `--spec` document is
                            // still usable rather than silently unaddressable.
                            None => Param {
                                name: name.clone(),
                                required: true,
                                scalar: Scalar::Str,
                                choices: None,
                                help: format!("{name} path parameter (undeclared in the document)"),
                            },
                        }
                    })
                    .collect();

                let query_params = declared
                    .iter()
                    .filter(|p| p.location == "query")
                    .map(|p| param(doc, p, &p.name))
                    .collect();

                let (body_fields, opaque_body, has_body) = match raw
                    .request_body
                    .as_ref()
                    .and_then(|b| b.content.get("application/json"))
                {
                    Some(media) => {
                        let (fields, opaque) = body_fields(doc, &media.schema);
                        (fields, opaque, true)
                    }
                    None => (Vec::new(), false, false),
                };

                let credentials = doc.credentials(&raw);

                ops.push(Op {
                    api: index,
                    operation_id,
                    method,
                    path,
                    verb,
                    resource,
                    summary: raw
                        .summary
                        .or(raw.description)
                        .unwrap_or_default()
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                    path_params,
                    query_params,
                    body_fields,
                    opaque_body,
                    has_body,
                    credentials,
                });
            }
        }

        Self { apis, ops }
    }

    /// Operations grouped by `(verb, resource)` — one clap subcommand pair per
    /// group. Values are indices into `self.ops`, ordered by path-param count
    /// so the arity match below is deterministic.
    pub fn groups(&self) -> BTreeMap<(String, String), Vec<usize>> {
        let mut grouped: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
        for (index, op) in self.ops.iter().enumerate() {
            grouped
                .entry((op.verb.clone(), op.resource.clone()))
                .or_default()
                .push(index);
        }
        for indices in grouped.values_mut() {
            indices.sort_by_key(|i| self.ops[*i].arity());
        }
        grouped
    }

    /// Pick the operation in a group that takes exactly `supplied` positional
    /// arguments. This is what makes `x get todos` and `x get todos <id>` two
    /// different operations behind one command.
    pub fn select(
        &self,
        group: &[usize],
        supplied: usize,
        wanted_op: Option<&str>,
        wanted_api: Option<&str>,
    ) -> eyre::Result<usize> {
        let mut candidates: Vec<usize> = group
            .iter()
            .copied()
            .filter(|i| self.ops[*i].arity() == supplied)
            .collect();

        if let Some(id) = wanted_op {
            candidates.retain(|i| self.ops[*i].operation_id == id);
        }
        if let Some(api) = wanted_api {
            candidates.retain(|i| self.apis[self.ops[*i].api].key == api);
        }

        match candidates.as_slice() {
            [only] => Ok(*only),
            [] => {
                let arities: Vec<String> = group
                    .iter()
                    .map(|i| {
                        let op = &self.ops[*i];
                        format!("{} ({} argument(s))", op.operation_id, op.arity())
                    })
                    .collect();
                Err(eyre::eyre!(
                    "no operation here takes {supplied} positional argument(s); available: {}",
                    arities.join(", ")
                ))
            }
            many => {
                let ids: Vec<String> = many
                    .iter()
                    .map(|i| {
                        let op = &self.ops[*i];
                        format!("{} (--api {})", op.operation_id, self.apis[op.api].key)
                    })
                    .collect();
                Err(eyre::eyre!(
                    "ambiguous: {} — disambiguate with --op or --api",
                    ids.join(", ")
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        let todo = Doc::parse(
            "todo",
            r#"{
              "openapi": "3.1.0",
              "servers": [{"url": "/api"}],
              "paths": {
                "/todos": {
                  "get": {"operationId": "list_todos", "tags": ["todos"],
                          "parameters": [{"name": "limit", "in": "query",
                                          "schema": {"type": "integer"}}]},
                  "post": {"operationId": "create_todo", "tags": ["todos"],
                           "requestBody": {"required": true, "content": {"application/json":
                             {"schema": {"type": "object", "required": ["title"],
                              "properties": {"title": {"type": "string"}}}}}}}
                },
                "/todos/{id}": {
                  "get": {"operationId": "get_todo", "tags": ["todos"],
                          "parameters": [{"name": "id", "in": "path", "required": true,
                                          "schema": {"type": "string", "format": "uuid"}}]}
                },
                "/todos/{id}/complete": {
                  "post": {"operationId": "complete_todo", "tags": ["todos"],
                           "parameters": [{"name": "id", "in": "path", "required": true,
                                           "schema": {"type": "string"}}]}
                }
              }
            }"#,
        )
        .expect("fixture parses");

        Registry::build(vec![Api {
            key: "todo".to_owned(),
            origin: "http://127.0.0.1:8080".to_owned(),
            doc: todo,
        }])
    }

    #[test]
    fn list_and_get_collapse_into_one_verb_split_by_arity() {
        let registry = registry();
        let groups = registry.groups();
        let get = &groups[&("get".to_owned(), "todos".to_owned())];
        assert_eq!(get.len(), 2, "list_todos and get_todo share `x get todos`");

        let listing = registry
            .select(get, 0, None, None)
            .expect("no-arg selects list");
        assert_eq!(registry.ops[listing].operation_id, "list_todos");

        let single = registry
            .select(get, 1, None, None)
            .expect("one arg selects get");
        assert_eq!(registry.ops[single].operation_id, "get_todo");
    }

    #[test]
    fn a_custom_operation_keeps_its_own_verb() {
        // `complete_todo` must not be folded into create/update; a lifecycle
        // transition is its own command or it is unreachable.
        let registry = registry();
        let groups = registry.groups();
        let complete = &groups[&("complete".to_owned(), "todos".to_owned())];
        let index = registry.select(complete, 1, None, None).expect("selects");
        assert_eq!(registry.ops[index].operation_id, "complete_todo");
        assert_eq!(registry.ops[index].method, "post");
    }

    #[test]
    fn wrong_argument_count_names_the_alternatives() {
        let registry = registry();
        let groups = registry.groups();
        let get = &groups[&("get".to_owned(), "todos".to_owned())];
        let err = registry
            .select(get, 2, None, None)
            .expect_err("no 2-arg op");
        let message = err.to_string();
        assert!(message.contains("list_todos"), "{message}");
        assert!(message.contains("get_todo"), "{message}");
    }

    #[test]
    fn body_object_becomes_flags_and_required_is_carried() {
        let registry = registry();
        let create = registry
            .ops
            .iter()
            .find(|o| o.operation_id == "create_todo")
            .expect("present");
        assert!(!create.opaque_body);
        let title = &create.body_fields[0];
        assert_eq!(title.name, "title");
        assert!(title.required);
    }

    #[test]
    fn url_joins_origin_mount_and_path_without_doubling_slashes() {
        let registry = registry();
        assert_eq!(
            registry.apis[0].url("/todos/{id}"),
            "http://127.0.0.1:8080/api/todos/{id}"
        );
    }

    #[test]
    fn title_case_tags_become_typeable_resource_names() {
        // `SeaOrmResource` derives Title Case tags from the table name, so
        // zerg's document really does say `Projects` and `Cloud Resources`.
        // Without this, the command would be `x get "Cloud Resources"`.
        assert_eq!(normalize_resource("Projects"), "projects");
        assert_eq!(normalize_resource("Cloud Resources"), "cloud-resources");
        // Already-correct tags must pass through untouched, or every other
        // resource name changes.
        assert_eq!(normalize_resource("todos"), "todos");
        assert_eq!(normalize_resource("cloud-resources"), "cloud-resources");
    }
}
