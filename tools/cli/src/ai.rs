//! AI-assisted scaffolding. Turns a natural-language service description into a
//! structured `AiSpec`, then hands it to the deterministic scaffolder. The LLM
//! only proposes the spec; all file generation stays deterministic and testable.
//!
//! Requires `ANTHROPIC_API_KEY`. Model overridable via `ZERGCTL_MODEL`.

use eyre::{Result, eyre};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::scaffold::{AppKind, AppSpec};

const DEFAULT_MODEL: &str = "claude-sonnet-4-5";
const SYSTEM_PROMPT: &str = "You convert a software service description into a JSON spec for scaffolding a new app in a Rust/Nx monorepo. \
Respond with ONLY a JSON object, no prose, no code fences. Schema: \
{\"slug\": string (kebab-case app name, no spaces), \"kind\": \"rust\" | \"static\", \"port\": integer}. \
Use \"rust\" for backend/worker/API services and \"static\" for web frontends. \
Pick a sensible default port (8080 for http apps, 50051 for grpc).";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AiSpec {
    pub slug: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_kind() -> String {
    "rust".to_string()
}
fn default_port() -> u16 {
    8080
}

impl AiSpec {
    pub fn into_app_spec(
        self,
        kind_override: Option<AppKind>,
        conv: &crate::config::Conventions,
    ) -> Result<AppSpec> {
        let kind = match kind_override {
            Some(k) => k,
            None => match self.kind.as_str() {
                "rust" => AppKind::Rust,
                "static" => AppKind::Static,
                other => return Err(eyre!("unknown kind from model: {other:?}")),
            },
        };
        Ok(AppSpec::from_slug(&self.slug, kind, self.port, conv))
    }
}

/// Extract the first balanced-looking JSON object from arbitrary model text
/// (tolerates code fences / surrounding prose) and parse it into an `AiSpec`.
pub fn parse_spec(text: &str) -> Result<AiSpec> {
    let start = text
        .find('{')
        .ok_or_else(|| eyre!("model output contained no JSON object"))?;
    let end = text
        .rfind('}')
        .ok_or_else(|| eyre!("model output contained no JSON object"))?;
    if end < start {
        return Err(eyre!("model output contained no JSON object"));
    }
    let spec: AiSpec = serde_json::from_str(&text[start..=end])
        .map_err(|e| eyre!("could not parse spec JSON: {e}"))?;
    Ok(spec)
}

/// Pull the assistant text out of an Anthropic Messages API response body.
fn text_from_response(body: &Value) -> Result<String> {
    body.get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b.get("type") == Some(&json!("text")))
        })
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| eyre!("no text block in Anthropic response: {body}"))
}

/// Call the Anthropic Messages API to turn a prompt into an `AiSpec`.
pub async fn extract_spec(prompt: &str) -> Result<AiSpec> {
    let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
        eyre!("ANTHROPIC_API_KEY is not set — required for `zergctl ai` (use `zergctl new` for offline scaffolding)")
    })?;
    let model = std::env::var("ZERGCTL_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());

    let body = json!({
        "model": model,
        "max_tokens": 512,
        "system": SYSTEM_PROMPT,
        "messages": [{ "role": "user", "content": prompt }]
    });

    let resp = reqwest::Client::new()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(eyre!("Anthropic API error ({status}): {v}"));
    }
    parse_spec(&text_from_response(&v)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_json() {
        let s = parse_spec(r#"{"slug":"notifier","kind":"rust","port":50051}"#).unwrap();
        assert_eq!(s.slug, "notifier");
        assert_eq!(s.kind, "rust");
        assert_eq!(s.port, 50051);
    }

    #[test]
    fn parses_json_wrapped_in_fences_and_prose() {
        let text = "Here is the spec:\n```json\n{\"slug\": \"dashboard\", \"kind\": \"static\"}\n```\nDone.";
        let s = parse_spec(text).unwrap();
        assert_eq!(s.slug, "dashboard");
        assert_eq!(s.kind, "static");
        assert_eq!(s.port, 8080); // default
    }

    #[test]
    fn missing_json_is_an_error() {
        assert!(parse_spec("no json here").is_err());
    }

    #[test]
    fn into_app_spec_maps_kind_and_respects_override() {
        let spec = AiSpec {
            slug: "email-blast".into(),
            kind: "rust".into(),
            port: 8080,
        };
        let conv = crate::config::Conventions::zerg();
        let app = spec.clone().into_app_spec(None, &conv).unwrap();
        assert_eq!(app.kind, AppKind::Rust);
        assert_eq!(app.name, "zerg_email_blast");

        let app2 = spec.into_app_spec(Some(AppKind::Static), &conv).unwrap();
        assert_eq!(app2.kind, AppKind::Static);
    }

    #[test]
    fn extracts_text_from_anthropic_body() {
        let body = json!({
            "content": [{ "type": "text", "text": "{\"slug\":\"x\"}" }]
        });
        assert_eq!(text_from_response(&body).unwrap(), "{\"slug\":\"x\"}");
    }
}
