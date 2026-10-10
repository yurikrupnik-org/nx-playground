//! A deliberately small OpenAPI 3.1 reader.
//!
//! This is not a general-purpose OpenAPI implementation and must not grow into
//! one. It reads exactly the subset that the documents in `docs/openapi/` —
//! produced by the `export_openapi_*` tests from this workspace's own
//! `#[utoipa::path]` annotations — actually use, and it is permissive about
//! everything else so an unknown keyword never breaks the CLI.
//!
//! The five invariants the documents guarantee (enforced by the invariant tests
//! next to each `export_openapi_*`) are what let the reader stay this small:
//! `openapi: 3.1.0`, path keys begin with `/`, every operation has a unique
//! `operationId` and a tag, every `{placeholder}` has a declared path
//! parameter, and error statuses name a body schema where one exists.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

/// HTTP methods a path item may carry. Anything else in a path item (`summary`,
/// `description`, `servers`, `parameters`) is read separately or ignored.
pub const METHODS: [&str; 7] = ["get", "put", "post", "delete", "patch", "head", "options"];

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Doc {
    #[serde(default)]
    pub info: Info,
    #[serde(default)]
    pub servers: Vec<Server>,
    /// Raw path items. Deserialized loosely because a path item mixes method
    /// keys with non-method keys; [`Doc::operations`] does the sorting out.
    #[serde(default)]
    pub paths: BTreeMap<String, BTreeMap<String, Value>>,
    #[serde(default)]
    pub components: Components,
    /// Document-wide fallback for operations that declare no `security` of
    /// their own. utoipa does not emit it, but a `--spec` document may.
    #[serde(default)]
    pub security: Option<Vec<Requirement>>,
}

/// One entry of an OpenAPI `security` list: scheme name -> scopes. An *empty*
/// map is the spelling for "this operation needs nothing", which is why the
/// list cannot be flattened to a set of names.
pub type Requirement = BTreeMap<String, Vec<String>>;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Info {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Server {
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Components {
    #[serde(default)]
    pub schemas: BTreeMap<String, Schema>,
    #[serde(default)]
    pub security_schemes: BTreeMap<String, SecurityScheme>,
}

/// A `securitySchemes` entry, read only as far as a client needs: which header
/// or cookie to put the credential in. `type: oauth2`/`openIdConnect` carry
/// flow metadata that cannot help a CLI acquire a token, so they are left as
/// [`Credential::Other`] and named in the error instead of half-supported.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SecurityScheme {
    #[serde(rename = "type", default)]
    pub ty: String,
    /// `apiKey` only: `cookie`, `header` or `query`.
    #[serde(rename = "in", default)]
    pub location: Option<String>,
    /// `apiKey` only: the cookie/header/query name.
    #[serde(default)]
    pub name: Option<String>,
    /// `http` only: `bearer`, `basic`, ...
    #[serde(default)]
    pub scheme: Option<String>,
}

/// The credential an operation declares, reduced to what a client must send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// `Authorization: Bearer <value>`.
    Bearer,
    /// `Cookie: <name>=<value>`.
    Cookie(String),
    /// A scheme this reader does not implement, kept by name so the error can
    /// say what the document asked for rather than "unauthorized".
    Other(String),
}

impl Credential {
    /// The credential, named the way the document names it.
    pub fn label(&self) -> String {
        match self {
            Self::Bearer => "bearer token".to_owned(),
            Self::Cookie(name) => format!("session cookie `{name}`"),
            Self::Other(scheme) => format!("`{scheme}` scheme"),
        }
    }

    /// The label plus the flag that supplies it, for errors and `--dry-run`.
    pub fn describe(&self) -> String {
        match self {
            Self::Bearer => "bearer token (--token, $X_TOKEN)".to_owned(),
            Self::Cookie(name) => format!("session cookie `{name}` (--session, $X_SESSION)"),
            Self::Other(scheme) => {
                format!("`{scheme}` scheme, which x cannot send; use --header")
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RawOperation {
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<RawParam>,
    #[serde(default)]
    pub request_body: Option<RawBody>,
    /// `None` means "not stated", which falls back to the document's
    /// `security`; `Some([])` means the operation states it needs nothing.
    #[serde(default)]
    pub security: Option<Vec<Requirement>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RawParam {
    pub name: String,
    #[serde(rename = "in")]
    pub location: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub schema: Schema,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RawBody {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub content: BTreeMap<String, MediaType>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MediaType {
    #[serde(default)]
    pub schema: Schema,
}

/// OpenAPI 3.1 allows `type` to be a single string or an array of strings
/// (`["string", "null"]` is how utoipa spells an `Option<String>`).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TypeSpec {
    One(String),
    Many(Vec<String>),
}

impl TypeSpec {
    /// The first non-`null` type name, which is the one a CLI must coerce to.
    pub fn primary(&self) -> Option<&str> {
        match self {
            Self::One(t) => Some(t.as_str()),
            Self::Many(ts) => ts.iter().find(|t| *t != "null").map(String::as_str),
        }
    }

    pub fn nullable(&self) -> bool {
        matches!(self, Self::Many(ts) if ts.iter().any(|t| t == "null"))
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Schema {
    #[serde(rename = "$ref", default)]
    pub reference: Option<String>,
    #[serde(rename = "type", default)]
    pub ty: Option<TypeSpec>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(rename = "enum", default)]
    pub enumeration: Option<Vec<Value>>,
    #[serde(default)]
    pub items: Option<Box<Schema>>,
    #[serde(default)]
    pub properties: BTreeMap<String, Schema>,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub all_of: Vec<Schema>,
    #[serde(default)]
    pub one_of: Vec<Schema>,
    #[serde(default)]
    pub any_of: Vec<Schema>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default: Option<Value>,
}

/// How a scalar value on the command line must be encoded into JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    Str,
    Int,
    Num,
    Bool,
    /// Object, array, or anything the reader could not pin down: the value is
    /// parsed as JSON and passed through, so `--tags '["a","b"]'` works.
    Json,
}

impl Doc {
    pub fn parse(source: &str, raw: &str) -> eyre::Result<Self> {
        serde_json::from_str(raw)
            .map_err(|e| eyre::eyre!("{source}: not a readable OpenAPI document: {e}"))
    }

    /// The mount path the document's operations hang off, relative to the
    /// process origin. `servers[0].url` in every document here is a path, not
    /// an absolute URL (`/api`, `/api/todos`) — an absolute one is honoured by
    /// [`crate::model::Api::origin`] instead.
    pub fn mount(&self) -> &str {
        self.servers
            .first()
            .map(|s| s.url.as_str())
            .unwrap_or_default()
    }

    /// What the document says an operation needs, resolved against
    /// `components.securitySchemes`.
    ///
    /// An empty result means "send nothing extra". OpenAPI's `security` list is
    /// alternatives (any one entry satisfies the operation) and each entry is a
    /// conjunction; a CLI cannot usefully act on that distinction, so every
    /// named scheme is returned and the caller sends whichever it holds. An
    /// empty requirement entry anywhere in the list makes the operation public,
    /// which is how a document exempts one route from a document-wide default.
    pub fn credentials(&self, op: &RawOperation) -> Vec<Credential> {
        let Some(requirements) = op.security.as_ref().or(self.security.as_ref()) else {
            return Vec::new();
        };
        if requirements.iter().any(BTreeMap::is_empty) {
            return Vec::new();
        }

        let mut out: Vec<Credential> = Vec::new();
        for name in requirements.iter().flat_map(BTreeMap::keys) {
            let credential = match self.components.security_schemes.get(name) {
                Some(scheme) => match scheme.ty.as_str() {
                    "apiKey" if scheme.location.as_deref() == Some("cookie") => scheme
                        .name
                        .clone()
                        .map_or_else(|| Credential::Other(name.clone()), Credential::Cookie),
                    "http" if scheme.scheme.as_deref() == Some("bearer") => Credential::Bearer,
                    _ => Credential::Other(name.clone()),
                },
                // A requirement naming an undeclared scheme: the document is
                // wrong, but it still says the route is guarded.
                None => Credential::Other(name.clone()),
            };
            if !out.contains(&credential) {
                out.push(credential);
            }
        }
        out
    }

    /// Every `(path, method, operation)` triple, in document order.
    pub fn operations(&self) -> Vec<(String, String, RawOperation, Vec<RawParam>)> {
        let mut out = Vec::new();
        for (path, item) in &self.paths {
            // Parameters declared on the path item apply to every operation in
            // it. utoipa never emits these, but a document fetched from a
            // third-party server with `--spec` may.
            let shared: Vec<RawParam> = item
                .get("parameters")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            for method in METHODS {
                let Some(raw) = item.get(method) else {
                    continue;
                };
                let Ok(op) = serde_json::from_value::<RawOperation>(raw.clone()) else {
                    continue;
                };
                out.push((path.clone(), method.to_string(), op, shared.clone()));
            }
        }
        out
    }

    /// Follow `$ref` and collapse the single-branch `allOf`/`oneOf`/`anyOf`
    /// wrappers utoipa emits for newtypes and `Option<T>`. Bounded to avoid a
    /// cycle in a hand-written document loaded via `--spec`.
    pub fn resolve<'a>(&'a self, schema: &'a Schema) -> &'a Schema {
        let mut current = schema;
        for _ in 0..16 {
            if let Some(reference) = &current.reference {
                let name = reference.rsplit('/').next().unwrap_or_default();
                match self.components.schemas.get(name) {
                    Some(next) => {
                        current = next;
                        continue;
                    }
                    None => return current,
                }
            }
            // `oneOf: [{type: null}, X]` is how an optional field arrives.
            let branches = if !current.all_of.is_empty() {
                &current.all_of
            } else if !current.one_of.is_empty() {
                &current.one_of
            } else if !current.any_of.is_empty() {
                &current.any_of
            } else {
                return current;
            };
            match branches
                .iter()
                .find(|b| !matches!(b.ty.as_ref().and_then(TypeSpec::primary), Some("null")))
            {
                Some(next) => current = next,
                None => return current,
            }
        }
        current
    }

    /// The CLI-level type of a schema, after resolution.
    pub fn scalar(&self, schema: &Schema) -> Scalar {
        let resolved = self.resolve(schema);
        match resolved.ty.as_ref().and_then(TypeSpec::primary) {
            Some("string") => Scalar::Str,
            Some("integer") => Scalar::Int,
            Some("number") => Scalar::Num,
            Some("boolean") => Scalar::Bool,
            // An enum with no declared type is still a set of strings here.
            None if resolved.enumeration.is_some() => Scalar::Str,
            _ => Scalar::Json,
        }
    }

    /// The allowed string values of an enum schema, for clap's value parser and
    /// its generated help.
    pub fn choices(&self, schema: &Schema) -> Option<Vec<String>> {
        let resolved = self.resolve(schema);
        let values = resolved.enumeration.as_ref()?;
        let choices: Vec<String> = values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        (!choices.is_empty()).then_some(choices)
    }
}

/// Coerce one command-line string into JSON according to the schema's type.
/// A value that does not fit is an error, not a silent string — sending
/// `{"limit": "ten"}` to a server expecting an integer is a 400 the CLI can
/// and should catch first.
pub fn coerce(name: &str, raw: &str, scalar: Scalar) -> eyre::Result<Value> {
    let value = match scalar {
        Scalar::Str => Value::String(raw.to_owned()),
        Scalar::Int => Value::from(
            raw.parse::<i64>()
                .map_err(|_| eyre::eyre!("--{name} expects an integer, got `{raw}`"))?,
        ),
        Scalar::Num => Value::from(
            raw.parse::<f64>()
                .map_err(|_| eyre::eyre!("--{name} expects a number, got `{raw}`"))?,
        ),
        Scalar::Bool => Value::Bool(
            raw.parse::<bool>()
                .map_err(|_| eyre::eyre!("--{name} expects true or false, got `{raw}`"))?,
        ),
        Scalar::Json => serde_json::from_str(raw)
            .map_err(|e| eyre::eyre!("--{name} expects JSON, got `{raw}`: {e}"))?,
    };
    Ok(value)
}

/// The query-string rendering of a coerced value. Scalars go in bare; anything
/// structured goes in as JSON, which is what `serde_urlencoded` on the axum
/// side will reject loudly rather than misread.
pub fn query_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Doc {
        Doc::parse(
            "test",
            r##"{
              "openapi": "3.1.0",
              "paths": {
                "/todos": {
                  "get": {
                    "operationId": "list_todos",
                    "tags": ["todos"],
                    "parameters": [
                      {"name": "limit", "in": "query", "required": false,
                       "schema": {"type": ["integer", "null"], "format": "int64"}},
                      {"name": "priority", "in": "query", "required": false,
                       "schema": {"$ref": "#/components/schemas/TodoPriority"}}
                    ]
                  },
                  "post": {
                    "operationId": "create_todo",
                    "tags": ["todos"],
                    "requestBody": {
                      "required": true,
                      "content": {"application/json": {
                        "schema": {"$ref": "#/components/schemas/CreateTodo"}}}
                    }
                  }
                }
              },
              "components": {"schemas": {
                "TodoPriority": {"type": "string", "enum": ["low", "medium", "high"]},
                "CreateTodo": {
                  "type": "object",
                  "required": ["title"],
                  "properties": {
                    "title": {"type": "string"},
                    "completed": {"type": "boolean"},
                    "priority": {"oneOf": [{"type": "null"},
                                 {"$ref": "#/components/schemas/TodoPriority"}]}
                  }
                }
              }}
            }"##,
        )
        .expect("fixture parses")
    }

    #[test]
    fn nullable_integer_is_an_integer_not_a_string() {
        // `Option<i64>` arrives as `type: ["integer","null"]`. Treating that as
        // opaque JSON would send `"limit": "50"` and earn a 400.
        let doc = doc();
        let op = &doc.operations()[0].2;
        let limit = &op.parameters[0].schema;
        assert_eq!(doc.scalar(limit), Scalar::Int);
        assert!(limit.ty.as_ref().expect("typed").nullable());
    }

    #[test]
    fn enum_choices_survive_a_ref_and_a_null_branch() {
        // Both the bare `$ref` (query param) and the `oneOf[null, $ref]`
        // (optional body field) must yield the same three choices, or the CLI
        // offers validation on one and not the other.
        let doc = doc();
        let operations = doc.operations();
        let via_ref = &operations[0].2.parameters[1].schema;
        let body = doc.resolve(
            &operations[1].2.request_body.as_ref().expect("body").content["application/json"]
                .schema,
        );
        let via_one_of = &body.properties["priority"];

        let expected = vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()];
        assert_eq!(doc.choices(via_ref), Some(expected.clone()));
        assert_eq!(doc.choices(via_one_of), Some(expected));
    }

    #[test]
    fn coercion_rejects_a_value_the_server_would_reject() {
        assert!(coerce("limit", "ten", Scalar::Int).is_err());
        assert_eq!(
            coerce("completed", "true", Scalar::Bool).expect("parses"),
            Value::Bool(true)
        );
        assert_eq!(
            coerce("limit", "50", Scalar::Int).expect("parses"),
            Value::from(50)
        );
    }

    /// A document that guards three routes differently: the cookie every
    /// service here declares, a bearer scheme, a scheme the reader cannot
    /// send, and an explicit public exemption under a document-wide default.
    fn guarded_doc() -> Doc {
        Doc::parse(
            "guarded",
            r##"{
              "openapi": "3.1.0",
              "security": [{"session_cookie": []}],
              "paths": {
                "/assets": {
                  "get": {"operationId": "list_assets", "tags": ["assets"],
                          "security": [{"session_cookie": []}]},
                  "post": {"operationId": "create_asset", "tags": ["assets"],
                           "security": [{"bearer_auth": []}, {"oauth2": ["write"]}]}
                },
                "/auth/login/password": {
                  "post": {"operationId": "password_login", "tags": ["auth"],
                           "security": [{}]}
                },
                "/health": {
                  "get": {"operationId": "health", "tags": ["health"]}
                }
              },
              "components": {"securitySchemes": {
                "session_cookie": {"type": "apiKey", "in": "cookie", "name": "terran_session"},
                "bearer_auth": {"type": "http", "scheme": "bearer"},
                "oauth2": {"type": "oauth2", "flows": {}}
              }}
            }"##,
        )
        .expect("fixture parses")
    }

    #[test]
    fn credentials_name_the_cookie_the_document_declares() {
        let doc = guarded_doc();
        let by_id: BTreeMap<String, RawOperation> = doc
            .operations()
            .into_iter()
            .map(|(_, _, op, _)| (op.operation_id.clone().expect("id"), op))
            .collect();

        // The cookie *name* is what a client needs; the scheme name is not it.
        assert_eq!(
            doc.credentials(&by_id["list_assets"]),
            vec![Credential::Cookie("terran_session".to_owned())]
        );
        // Alternatives are all reported, including one x cannot send, so the
        // error can say what the document wanted.
        assert_eq!(
            doc.credentials(&by_id["create_asset"]),
            vec![Credential::Bearer, Credential::Other("oauth2".to_owned())]
        );
        // `security: [{}]` is the spelling for "public", and it has to beat the
        // document-wide default or every login route becomes unreachable.
        assert!(doc.credentials(&by_id["password_login"]).is_empty());
        // No operation-level `security` at all: the document default applies.
        assert_eq!(
            doc.credentials(&by_id["health"]),
            vec![Credential::Cookie("terran_session".to_owned())]
        );
    }
}
