//! Server-side JSON client for todo-api — the Rust twin of the Astro
//! variant's `src/lib/api.ts`. Never shipped to the browser.
//!
//! The DTOs mirror only the fields this frontend renders; serde ignores the
//! rest. Wire shape is owned by `domain_todo::models` (`Todo`, `CreateTodo`,
//! `TodoPriority` — serde lowercase) — keep in sync.
//!
//! Feature flags are resolved by todo-api (`GET /api/flags`), never by this
//! app: one evaluation point, one identity resolution order. Every upstream
//! call carries the identity headers so todo-api can target per user.

use std::collections::BTreeMap;
use std::time::Duration;

use eyre::{Result, WrapErr, eyre};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

/// Priorities in form/render order (mirrors `PRIORITIES` in fragments.ts).
pub const PRIORITIES: [Priority; 3] = [Priority::Low, Priority::Medium, Priority::High];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Medium,
    High,
}

impl Priority {
    /// Wire/CSS token: `badge--{low,medium,high}`, form option values.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Parse a form value; `None` for anything not in [`PRIORITIES`].
    pub fn parse(value: &str) -> Option<Self> {
        PRIORITIES.into_iter().find(|p| p.as_str() == value)
    }
}

/// The slice of a todo this frontend renders.
#[derive(Clone, Debug, Deserialize)]
pub struct Todo {
    pub id: String,
    pub title: String,
    pub completed: bool,
    pub priority: Priority,
}

#[derive(Debug, Serialize)]
pub struct CreateTodo {
    pub title: String,
    pub description: String,
    pub priority: Priority,
}

/// The flag catalogue in display order. All six are always sent by todo-api;
/// unknown extra keys are carried along and ignored.
pub const KNOWN_FLAGS: [&str; 6] = [
    "todo_app_web",
    "todo_app_htmx",
    "todo_app_astro",
    "todo_realtime",
    "todo_write",
    "todo_max_items",
];

/// This app's kill switch: off ⇒ every route answers 503.
pub const FLAG_APP: &str = "todo_app_htmx";
/// Mutations allowed: off ⇒ read-only UI and 403 on the mutating partials.
pub const FLAG_WRITE: &str = "todo_write";
/// Todo count cap. Enforced by todo-api (429 on create); displayed here.
pub const FLAG_MAX_ITEMS: &str = "todo_max_items";
/// [`FLAG_MAX_ITEMS`] value meaning "no cap" — the catalogue default.
pub const UNLIMITED: i64 = -1;

/// `source` when the flags were not resolved against Flagsmith (unconfigured
/// or unreachable). Informational, not an error.
pub const SOURCE_DEFAULTS: &str = "defaults";

/// One flag as todo-api reports it. `value` is a passthrough of Flagsmith's
/// `feature_state_value` (null / number / string).
#[derive(Clone, Debug, Deserialize)]
pub struct FlagState {
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub value: Value,
}

/// Fail open: a flag whose `enabled` is missing counts as ON.
fn enabled_default() -> bool {
    true
}

fn source_default() -> String {
    SOURCE_DEFAULTS.to_owned()
}

/// The `GET /api/flags` payload. `identity` is echoed by todo-api but this app
/// already knows its own, so it is ignored here.
#[derive(Clone, Debug, Deserialize)]
pub struct Flags {
    #[serde(default = "source_default")]
    source: String,
    #[serde(default)]
    flags: BTreeMap<String, FlagState>,
}

impl Flags {
    /// The degraded state: every flag ON, `source = "defaults"`. Used whenever
    /// the flag fetch fails — the product must stay usable.
    pub fn defaults() -> Self {
        let flags = KNOWN_FLAGS
            .into_iter()
            .map(|flag| {
                let value = if flag == FLAG_MAX_ITEMS {
                    Value::from(UNLIMITED)
                } else {
                    Value::Null
                };
                (
                    flag.to_owned(),
                    FlagState {
                        enabled: true,
                        value,
                    },
                )
            })
            .collect();
        Self {
            source: source_default(),
            flags,
        }
    }

    /// `"remote"` or `"defaults"`; rendered in the status strip.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Missing keys read as ON — an incomplete payload must never hide the UI.
    pub fn enabled(&self, flag: &str) -> bool {
        self.flags.get(flag).is_none_or(|state| state.enabled)
    }

    /// Numeric flag value; accepts a JSON number or a numeric string.
    pub fn int(&self, flag: &str) -> Option<i64> {
        match &self.flags.get(flag)?.value {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        }
    }
}

/// Identity forwarding: todo-api turns these into the Flagsmith identity and
/// its `app` trait, which is what makes "user X on app Y" targeting work.
const APP_HEADER: &str = "X-Todo-App";
const IDENTITY_HEADER: &str = "X-Todo-Identity";
const APP_NAME: &str = "htmx";

/// Flags must never stall a render: on timeout we fall back to the defaults.
const FLAGS_TIMEOUT: Duration = Duration::from_secs(2);

/// JSON client bound to one todo-api origin. Cheap to clone (reqwest `Client`
/// is an `Arc` internally).
#[derive(Clone)]
pub struct TodoApi {
    http: Client,
    origin: String,
}

impl TodoApi {
    pub fn new(origin: String) -> Self {
        Self {
            http: Client::new(),
            origin: origin.trim_end_matches('/').to_owned(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/todos{path}", self.origin)
    }

    /// Stamp the app + identity headers on every outgoing request.
    fn tagged(request: RequestBuilder, identity: Option<&str>) -> RequestBuilder {
        let request = request.header(APP_HEADER, APP_NAME);
        match identity {
            Some(identity) => request.header(IDENTITY_HEADER, identity),
            None => request,
        }
    }

    /// Mirror of api.ts `expectOk`: non-2xx becomes an error carrying status + body.
    async fn expect_ok(response: Response) -> Result<Response> {
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(eyre!("todo-api {status}: {body}"));
        }
        Ok(response)
    }

    /// Flags for one identity as resolved by todo-api.
    pub async fn flags(&self, identity: Option<&str>) -> Result<Flags> {
        let request = self
            .http
            .get(format!("{}/api/flags", self.origin))
            .timeout(FLAGS_TIMEOUT);
        let response = Self::tagged(request, identity)
            .send()
            .await
            .wrap_err("todo-api unreachable")?;
        Ok(Self::expect_ok(response).await?.json().await?)
    }

    /// Infallible [`Self::flags`]: a broken flag source degrades to all-ON
    /// defaults instead of taking the page down.
    pub async fn flags_or_defaults(&self, identity: Option<&str>) -> Flags {
        match self.flags(identity).await {
            Ok(flags) => flags,
            Err(err) => {
                warn!(error = %err, "flag fetch failed; using defaults");
                Flags::defaults()
            }
        }
    }

    pub async fn list(&self, identity: Option<&str>) -> Result<Vec<Todo>> {
        let response = Self::tagged(self.http.get(self.url("?limit=100000")), identity)
            .send()
            .await
            .wrap_err("todo-api unreachable")?;
        Ok(Self::expect_ok(response).await?.json().await?)
    }

    /// `None` when the API answers 404 (unknown id — including non-UUID ids).
    pub async fn get(&self, id: &str, identity: Option<&str>) -> Result<Option<Todo>> {
        let response = Self::tagged(self.http.get(self.url(&format!("/{id}"))), identity)
            .send()
            .await
            .wrap_err("todo-api unreachable")?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(Self::expect_ok(response).await?.json().await?))
    }

    pub async fn create(&self, input: &CreateTodo, identity: Option<&str>) -> Result<Todo> {
        let response = Self::tagged(self.http.post(self.url("")).json(input), identity)
            .send()
            .await
            .wrap_err("todo-api unreachable")?;
        Ok(Self::expect_ok(response).await?.json().await?)
    }

    /// Flip completion given the currently rendered state (idempotent per state).
    pub async fn toggle(&self, id: &str, completed: bool, identity: Option<&str>) -> Result<Todo> {
        let action = if completed { "uncomplete" } else { "complete" };
        let response = Self::tagged(
            self.http.post(self.url(&format!("/{id}/{action}"))),
            identity,
        )
        .send()
        .await
        .wrap_err("todo-api unreachable")?;
        Ok(Self::expect_ok(response).await?.json().await?)
    }

    pub async fn remove(&self, id: &str, identity: Option<&str>) -> Result<()> {
        let response = Self::tagged(self.http.delete(self.url(&format!("/{id}"))), identity)
            .send()
            .await
            .wrap_err("todo-api unreachable")?;
        Self::expect_ok(response).await?;
        Ok(())
    }
}

// The flag payload is a wire contract with todo-api: fail-open defaults and the
// number-or-numeric-string tolerance are the load-bearing parts.
#[cfg(test)]
mod tests {
    use super::*;

    fn flags(json: &str) -> Flags {
        serde_json::from_str(json).expect("flag payload parses")
    }

    #[test]
    fn reads_the_documented_wire_format() {
        let parsed = flags(
            r#"{"identity":"yuri","source":"remote","flags":{
                 "todo_write":{"enabled":false,"value":null},
                 "todo_max_items":{"enabled":true,"value":5}}}"#,
        );
        assert_eq!(parsed.source(), "remote");
        assert!(!parsed.enabled(FLAG_WRITE));
        assert_eq!(parsed.int(FLAG_MAX_ITEMS), Some(5));
    }

    #[test]
    fn missing_flags_and_unknown_extras_fail_open() {
        let parsed = flags(r#"{"source":"remote","flags":{"brand_new_flag":{"enabled":false}}}"#);
        for flag in KNOWN_FLAGS {
            assert!(parsed.enabled(flag), "{flag} should default on");
        }
        assert!(!parsed.enabled("brand_new_flag"));
        assert_eq!(parsed.int(FLAG_MAX_ITEMS), None);
    }

    #[test]
    fn accepts_numeric_strings_for_int_flags() {
        let parsed = flags(
            r#"{"source":"remote","flags":{"todo_max_items":{"enabled":true,"value":" 12 "}}}"#,
        );
        assert_eq!(parsed.int(FLAG_MAX_ITEMS), Some(12));
        let bogus = flags(
            r#"{"source":"remote","flags":{"todo_max_items":{"enabled":true,"value":"lots"}}}"#,
        );
        assert_eq!(bogus.int(FLAG_MAX_ITEMS), None);
    }

    #[test]
    fn defaults_are_all_on_and_labelled() {
        let parsed = Flags::defaults();
        assert_eq!(parsed.source(), SOURCE_DEFAULTS);
        assert!(KNOWN_FLAGS.into_iter().all(|flag| parsed.enabled(flag)));
    }
}
