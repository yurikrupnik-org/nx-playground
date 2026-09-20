//! Request execution and error decoding.

use std::time::Duration;

use reqwest::{Client, Method, RequestBuilder, header};
use serde::Deserialize;
use serde_json::Value;

use crate::command::Invocation;
use crate::spec::Credential;

/// The structured error envelope this workspace's domain handlers return,
/// mirrored from `libs/core/axum-helpers/src/errors/mod.rs` (`ErrorResponse`).
///
/// It is redeclared rather than imported because `axum-helpers` pulls axum,
/// sea-orm and sqlx — a CLI cannot depend on it. The fields must stay in step
/// with that struct; the shape is also published in every OpenAPI document
/// here as the `ErrorResponse` component, which is the gate that catches a
/// divergence.
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    code: i32,
    error: String,
    message: String,
    #[serde(default)]
    details: Option<Value>,
}

/// Everything about a response the renderer or an error report needs.
pub struct Outcome {
    pub status: u16,
    pub body: Option<Value>,
    /// Bytes actually received, for `--dry-run`-adjacent reporting and for the
    /// wire-cost numbers in the delivery-surface comparison.
    pub bytes: usize,
}

pub struct Transport {
    client: Client,
    token: Option<String>,
    session: Option<String>,
    headers: Vec<(String, String)>,
}

impl Transport {
    pub fn new(
        timeout: Duration,
        token: Option<String>,
        session: Option<String>,
        headers: Vec<(String, String)>,
    ) -> eyre::Result<Self> {
        let client = Client::builder()
            .timeout(timeout)
            .user_agent(concat!("x/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            token,
            session,
            headers,
        })
    }

    /// Attach the credential the operation's document asks for, and report
    /// which one went out so a rejection can name it.
    ///
    /// A held bearer token wins even where the document declares only the
    /// browser cookie: every guarded route in this workspace accepts either
    /// (`libs/core/oidc-auth/src/middleware.rs`, `extract_credentials`), and
    /// the documents describe the browser path because that is the one the
    /// SPAs use.
    fn authorize(
        &self,
        request: RequestBuilder,
        invocation: &Invocation,
    ) -> eyre::Result<(RequestBuilder, Option<Credential>)> {
        if let Some(token) = &self.token {
            return Ok((request.bearer_auth(token), Some(Credential::Bearer)));
        }

        let cookie_name = invocation.credentials.iter().find_map(|c| match c {
            Credential::Cookie(name) => Some(name.clone()),
            _ => None,
        });
        if let (Some(session), Some(name)) = (&self.session, cookie_name) {
            let request = request.header(header::COOKIE, format!("{name}={session}"));
            return Ok((request, Some(Credential::Cookie(name))));
        }

        if invocation.credentials.is_empty() {
            return Ok((request, None));
        }

        // Fail before the round trip: the document already said this would be
        // a 401, and it named the credential that would fix it.
        let declared: Vec<String> = invocation
            .credentials
            .iter()
            .map(Credential::describe)
            .collect();
        Err(eyre::eyre!(
            "{} {} needs a credential\n  the document declares: {}\n  --token/$X_TOKEN is sent as `Authorization: Bearer`; \
             --session/$X_SESSION is sent as the declared cookie",
            invocation.method,
            invocation.url,
            declared.join(", ")
        ))
    }

    pub async fn send(&self, invocation: &Invocation) -> eyre::Result<Outcome> {
        let method = Method::from_bytes(invocation.method.as_bytes())
            .map_err(|_| eyre::eyre!("unsupported method {}", invocation.method))?;

        let mut request = self.client.request(method, &invocation.url);
        if !invocation.query.is_empty() {
            request = request.query(&invocation.query);
        }
        let (mut request, sent) = self.authorize(request, invocation)?;
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(body) = &invocation.body {
            request = request.json(body);
        }

        let response = request.send().await.map_err(|e| {
            // A connection failure is the single most common CLI outcome
            // against a service that is not running; say which URL, not just
            // "error sending request".
            eyre::eyre!("{} unreachable: {e}", invocation.url)
        })?;

        let status = response.status();
        let text = response.text().await?;
        let bytes = text.len();
        let body = if text.trim().is_empty() {
            None
        } else {
            serde_json::from_str(&text).ok().or_else(|| {
                // Two of the three error shapes in this workspace are plain
                // text (`zerg_api::error::ApiError`, `terran_api::error::
                // ApiError`) and `apps/todo/api/src/stacks.rs` returns a bare
                // string. Keep the text rather than discarding it.
                Some(Value::String(text.clone()))
            })
        };

        if !status.is_success() {
            let mut report = format!(
                "{} {}\n{}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("error"),
                describe_failure(body.as_ref())
            );
            if matches!(status.as_u16(), 401 | 403) {
                report.push('\n');
                report.push_str(&auth_hint(sent.as_ref()));
            }
            return Err(eyre::eyre!(report));
        }

        Ok(Outcome {
            status: status.as_u16(),
            body,
            bytes,
        })
    }
}

/// Render whichever of the three error shapes came back.
fn describe_failure(body: Option<&Value>) -> String {
    let Some(body) = body else {
        return "  (empty response body)".to_owned();
    };

    if let Ok(envelope) = serde_json::from_value::<ErrorEnvelope>(body.clone()) {
        let mut out = format!(
            "  {} ({}): {}",
            envelope.error, envelope.code, envelope.message
        );
        if let Some(details) = envelope.details {
            out.push_str(&format!(
                "\n  details: {}",
                serde_json::to_string_pretty(&details).unwrap_or_else(|_| details.to_string())
            ));
        }
        return out;
    }

    match body {
        Value::String(text) => format!("  {}", text.trim()),
        other => serde_json::to_string_pretty(other)
            .unwrap_or_else(|_| other.to_string())
            .lines()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The line appended to a 401/403. The three cases need different next steps:
/// a stateless token is rejected by the *verifier* (expired, wrong
/// realm/client), a session id is rejected by the *store* (expired, logged
/// out, different process), and no credential at all means the document is
/// lying about the route being public.
fn auth_hint(sent: Option<&Credential>) -> String {
    match sent {
        Some(Credential::Bearer) => "  x sent the bearer token (--token, $X_TOKEN) and the \
             service refused it: expired, or not issued by the realm/client it verifies"
            .to_owned(),
        Some(Credential::Cookie(name)) => format!(
            "  x sent the session cookie `{name}` (--session, $X_SESSION) and the service \
             refused it: expired, logged out, or belonging to another session store"
        ),
        Some(Credential::Other(scheme)) => {
            format!("  the document's `{scheme}` scheme was not satisfied")
        }
        None => "  x sent no credential: the document declares none for this operation, \
                 yet the route is guarded. Pass --token <JWT> ($X_TOKEN), or add the \
                 operation's `security(...)` to its #[utoipa::path] so x can tell"
            .to_owned(),
    }
}

/// Parse `NAME:VALUE`.
pub fn parse_header(raw: &str) -> eyre::Result<(String, String)> {
    let (name, value) = raw
        .split_once(':')
        .ok_or_else(|| eyre::eyre!("--header expects NAME:VALUE, got `{raw}`"))?;
    let name = name.trim();
    if name.is_empty() {
        return Err(eyre::eyre!("--header has an empty name: `{raw}`"));
    }
    header::HeaderName::try_from(name).map_err(|_| eyre::eyre!("invalid header name `{name}`"))?;
    Ok((name.to_owned(), value.trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_envelope_is_rendered_not_dumped() {
        // The domain handlers' shape: a reader should see the message, not
        // raw JSON.
        let body = serde_json::json!({
            "code": 1001,
            "error": "VALIDATION_ERROR",
            "message": "title must not be empty",
            "details": {"title": ["length"]}
        });
        let rendered = describe_failure(Some(&body));
        assert!(rendered.contains("VALIDATION_ERROR (1001): title must not be empty"));
        assert!(rendered.contains("details"));
    }

    #[test]
    fn plain_text_error_bodies_survive() {
        // zerg_api and terran_api return `(StatusCode, &'static str)`.
        let body = Value::String("forbidden\n".to_owned());
        assert_eq!(describe_failure(Some(&body)), "  forbidden");
    }

    #[test]
    fn header_parsing_rejects_what_reqwest_would_reject_later() {
        assert!(parse_header("x-todo-app").is_err());
        assert!(parse_header(" :value").is_err());
        assert!(parse_header("bad header:value").is_err());
        assert_eq!(
            parse_header("X-Todo-Identity: alice").expect("valid"),
            ("X-Todo-Identity".to_owned(), "alice".to_owned())
        );
    }
}
