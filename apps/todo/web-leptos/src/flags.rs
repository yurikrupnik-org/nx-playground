//! Feature flags, resolved by todo-api — the Rust twin of
//! `apps/todo/web/src/lib/flags.ts`.
//!
//! The SPA never talks to Flagsmith: it reads `GET /api/flags`, which todo-api
//! evaluates for the caller's identity. The response is advisory for the UI —
//! todo-api enforces the same flags server-side — so any failure here degrades
//! to "everything on" rather than blocking the app.

use std::collections::BTreeMap;

use gloo_net::http::Request;
use serde::Deserialize;
use serde_json::Value;

use crate::identity::TODO_APP;

const FLAGS_URL: &str = "/api/flags";

/// This app's kill switch. `"web"`, not a fourth catalogue entry — see
/// [`crate::identity::TODO_APP`].
pub const FLAG_APP: &str = "todo_app_web";
pub const FLAG_WRITE: &str = "todo_write";
pub const FLAG_REALTIME: &str = "todo_realtime";
pub const FLAG_MAX_ITEMS: &str = "todo_max_items";

/// [`FLAG_MAX_ITEMS`] value meaning "no cap" — the catalogue default.
pub const UNLIMITED: i64 = -1;

/// The six-flag catalogue todo-api always answers with.
const FLAG_NAMES: [&str; 6] = [
    "todo_app_web",
    "todo_app_htmx",
    "todo_app_astro",
    FLAG_REALTIME,
    FLAG_WRITE,
    FLAG_MAX_ITEMS,
];

/// One flag as todo-api reports it. `value` is a passthrough of Flagsmith's
/// `feature_state_value` (null / number / string).
#[derive(Debug, Clone, Deserialize)]
pub struct FlagState {
    /// Fail open, matching `normalize()` in flags.ts: an entry whose `enabled`
    /// is absent or not a boolean keeps the catalogue default, which is ON.
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub value: Value,
}

fn enabled_default() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct FlagsResponse {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub flags: BTreeMap<String, FlagState>,
}

impl FlagsResponse {
    /// Fail-open catalogue defaults: every app and feature on, no item cap.
    pub fn defaults() -> Self {
        let flags = FLAG_NAMES
            .into_iter()
            .map(|name| {
                let value = if name == FLAG_MAX_ITEMS {
                    Value::from(UNLIMITED)
                } else {
                    Value::Null
                };
                (
                    name.to_owned(),
                    FlagState {
                        enabled: true,
                        value,
                    },
                )
            })
            .collect();
        Self {
            source: "defaults".to_owned(),
            flags,
        }
    }

    /// Backfill anything the server omitted so all six catalogue flags are
    /// present, keeping unknown extra flags. Mirrors `normalize()` in flags.ts.
    fn normalized(mut self) -> Self {
        let mut merged = Self::defaults().flags;
        merged.append(&mut self.flags);
        self.flags = merged;
        if self.source != "remote" {
            self.source = "defaults".to_owned();
        }
        self
    }

    /// A flag's state, falling back to "on" for a key the server never sent.
    pub fn is_enabled(&self, name: &str) -> bool {
        self.flags.get(name).is_none_or(|state| state.enabled)
    }

    /// A flag's integer payload. Flagsmith returns `feature_state_value` as
    /// either a number or a string depending on how the value was typed in the
    /// dashboard, so both are accepted; anything else falls back.
    pub fn int_value(&self, name: &str, fallback: i64) -> i64 {
        match self.flags.get(name).map(|state| &state.value) {
            Some(Value::Number(n)) => n.as_f64().map_or(fallback, |f| f.trunc() as i64),
            Some(Value::String(s)) => s.trim().parse().unwrap_or(fallback),
            _ => fallback,
        }
    }
}

/// Fetch the flag set for the current identity. Never fails: a network error, a
/// non-2xx status or an unparseable body all yield the built-in defaults with
/// `source: "defaults"`.
pub async fn fetch(identity: &str) -> FlagsResponse {
    let Ok(response) = Request::get(FLAGS_URL)
        .header("X-Todo-Identity", identity)
        .header("X-Todo-App", TODO_APP)
        .send()
        .await
    else {
        return FlagsResponse::defaults();
    };
    if !response.ok() {
        return FlagsResponse::defaults();
    }
    response
        .json::<FlagsResponse>()
        .await
        .map_or_else(|_| FlagsResponse::defaults(), FlagsResponse::normalized)
}
