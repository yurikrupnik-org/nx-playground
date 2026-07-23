//! SendGrid email provider
//!
//! Sends emails via SendGrid HTTP API.

use crate::error::{NotificationError, NotificationResult};
use crate::models::Email;
use crate::provider::{EmailProvider, ProviderError, SendResult};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::Serialize;
use tracing::{debug, error};

/// SendGrid API endpoint
const SENDGRID_API_URL: &str = "https://api.sendgrid.com/v3/mail/send";

/// SendGrid email provider
pub struct SendGridProvider {
    api_key: String,
    from_email: String,
    from_name: String,
    client: Client,
}

impl SendGridProvider {
    /// Create a new SendGridProvider
    pub fn new(
        api_key: impl Into<String>,
        from_email: impl Into<String>,
        from_name: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            from_email: from_email.into(),
            from_name: from_name.into(),
            client: Client::new(),
        }
    }

    /// Create from environment variables
    ///
    /// Expects:
    /// - `SENDGRID_API_KEY`
    /// - `SENDGRID_FROM_EMAIL` or `EMAIL_FROM_ADDRESS`
    /// - `SENDGRID_FROM_NAME` or `EMAIL_FROM_NAME`
    pub fn from_env() -> NotificationResult<Self> {
        let api_key = std::env::var("SENDGRID_API_KEY")
            .map_err(|_| NotificationError::Config("SENDGRID_API_KEY not set".into()))?;

        let from_email = std::env::var("SENDGRID_FROM_EMAIL")
            .or_else(|_| std::env::var("EMAIL_FROM_ADDRESS"))
            .map_err(|_| {
                NotificationError::Config("SENDGRID_FROM_EMAIL or EMAIL_FROM_ADDRESS not set".into())
            })?;

        let from_name = std::env::var("SENDGRID_FROM_NAME")
            .or_else(|_| std::env::var("EMAIL_FROM_NAME"))
            .unwrap_or_else(|_| "Notifications".to_string());

        Ok(Self::new(api_key, from_email, from_name))
    }
}

/// SendGrid API request payload (borrows from the [`Email`] being sent)
#[derive(Debug, Serialize)]
struct SendGridRequest<'a> {
    personalizations: [Personalization<'a>; 1],
    from: EmailAddress<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<EmailAddress<'a>>,
    subject: &'a str,
    content: Vec<Content<'a>>,
}

#[derive(Debug, Serialize)]
struct Personalization<'a> {
    to: [EmailAddress<'a>; 1],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    cc: Vec<EmailAddress<'a>>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    bcc: Vec<EmailAddress<'a>>,
}

#[derive(Debug, Serialize)]
struct EmailAddress<'a> {
    email: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct Content<'a> {
    #[serde(rename = "type")]
    content_type: &'static str,
    value: &'a str,
}

/// Classify a SendGrid HTTP response status into a [`ProviderError`].
///
/// - 429 -> rate limited (with `Retry-After` hint when present)
/// - 408 and any 5xx -> transient
/// - remaining 4xx -> permanent
fn classify_status(status: StatusCode, retry_after_ms: Option<u64>, body: String) -> ProviderError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ProviderError::rate_limited(
            format!("SendGrid rate limited ({status}): {body}"),
            retry_after_ms,
        );
    }
    if status == StatusCode::REQUEST_TIMEOUT || status.is_server_error() {
        return ProviderError::transient(format!("SendGrid error ({status}): {body}"));
    }
    ProviderError::permanent(format!("SendGrid rejected request ({status}): {body}"))
}

#[async_trait]
impl EmailProvider for SendGridProvider {
    async fn send(&self, email: &Email) -> Result<SendResult, ProviderError> {
        // Build content
        let mut content = Vec::with_capacity(2);

        if let Some(text) = &email.body_text {
            content.push(Content {
                content_type: "text/plain",
                value: text,
            });
        }

        if let Some(html) = &email.body_html {
            content.push(Content {
                content_type: "text/html",
                value: html,
            });
        }

        if content.is_empty() {
            return Err(ProviderError::permanent(
                "Email must have text or HTML content",
            ));
        }

        fn to_address(addr: &str) -> EmailAddress<'_> {
            EmailAddress {
                email: addr,
                name: None,
            }
        }

        // Build request
        let request = SendGridRequest {
            personalizations: [Personalization {
                to: [EmailAddress {
                    email: &email.to,
                    name: None,
                }],
                cc: email.cc.iter().map(|a| to_address(a)).collect(),
                bcc: email.bcc.iter().map(|a| to_address(a)).collect(),
            }],
            from: EmailAddress {
                email: email.from.as_deref().unwrap_or(&self.from_email),
                name: Some(&self.from_name),
            },
            reply_to: email.reply_to.as_deref().map(|r| EmailAddress {
                email: r,
                name: None,
            }),
            subject: &email.subject,
            content,
        };

        debug!(
            to = %email.to,
            subject = %email.subject,
            "Sending email via SendGrid"
        );

        // Send request
        let response = self
            .client
            .post(SENDGRID_API_URL)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|e| ProviderError::transient_with_source("SendGrid request failed", e))?;

        let status = response.status();

        if status.is_success() {
            // SendGrid returns message ID in X-Message-Id header
            let message_id = response
                .headers()
                .get("X-Message-Id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(&email.id)
                .to_string();

            debug!(message_id = %message_id, "Email sent successfully");

            Ok(SendResult { message_id })
        } else {
            let retry_after_ms = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(|secs| secs * 1000);

            let error_body = response.text().await.unwrap_or_default();
            error!(
                status = %status,
                error = %error_body,
                "SendGrid API error"
            );

            Err(classify_status(status, retry_after_ms, error_body))
        }
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        // Simple validation that API key is set
        if self.api_key.is_empty() {
            return Err(ProviderError::permanent("SendGrid API key not configured"));
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "sendgrid"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_address_serialization() {
        let addr = EmailAddress {
            email: "test@example.com",
            name: Some("Test User"),
        };

        let json = serde_json::to_string(&addr).unwrap();
        assert!(json.contains("test@example.com"));
        assert!(json.contains("Test User"));
    }

    #[test]
    fn test_status_classification() {
        assert!(matches!(
            classify_status(StatusCode::TOO_MANY_REQUESTS, Some(2000), String::new()),
            ProviderError::RateLimited {
                retry_after_ms: Some(2000),
                ..
            }
        ));
        assert!(matches!(
            classify_status(StatusCode::REQUEST_TIMEOUT, None, String::new()),
            ProviderError::Transient { .. }
        ));
        assert!(matches!(
            classify_status(StatusCode::BAD_GATEWAY, None, String::new()),
            ProviderError::Transient { .. }
        ));
        assert!(matches!(
            classify_status(StatusCode::BAD_REQUEST, None, String::new()),
            ProviderError::Permanent { .. }
        ));
        assert!(matches!(
            classify_status(StatusCode::UNAUTHORIZED, None, String::new()),
            ProviderError::Permanent { .. }
        ));
    }
}
