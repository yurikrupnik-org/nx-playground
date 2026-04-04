//! AWS SES email provider
//!
//! Sends emails via AWS Simple Email Service (SES) v2 API.

use crate::models::Email;
use crate::provider::{EmailProvider, SendResult};
use async_trait::async_trait;
use aws_sdk_sesv2::types::{
    Body, Content, Destination, EmailContent, Message,
};
use eyre::{eyre, Result};
use tracing::{debug, error};

/// AWS SES email provider
pub struct SesProvider {
    client: aws_sdk_sesv2::Client,
    from_email: String,
    from_name: String,
}

impl SesProvider {
    /// Create a new SesProvider with an existing AWS SDK client
    pub fn new(
        client: aws_sdk_sesv2::Client,
        from_email: impl Into<String>,
        from_name: impl Into<String>,
    ) -> Self {
        Self {
            client,
            from_email: from_email.into(),
            from_name: from_name.into(),
        }
    }

    /// Create from environment variables and default AWS config
    ///
    /// Uses the default AWS credential chain (env vars, instance profile, etc.)
    ///
    /// Expects:
    /// - `SES_FROM_EMAIL` or `EMAIL_FROM_ADDRESS`
    /// - `SES_FROM_NAME` or `EMAIL_FROM_NAME` (optional, defaults to "Notifications")
    /// - Standard AWS env vars (`AWS_REGION`, `AWS_ACCESS_KEY_ID`, etc.) or instance profile
    pub async fn from_env() -> Result<Self> {
        let from_email = std::env::var("SES_FROM_EMAIL")
            .or_else(|_| std::env::var("EMAIL_FROM_ADDRESS"))
            .map_err(|_| eyre!("SES_FROM_EMAIL or EMAIL_FROM_ADDRESS not set"))?;

        let from_name = std::env::var("SES_FROM_NAME")
            .or_else(|_| std::env::var("EMAIL_FROM_NAME"))
            .unwrap_or_else(|_| "Notifications".to_string());

        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = aws_sdk_sesv2::Client::new(&config);

        Ok(Self::new(client, from_email, from_name))
    }

    fn format_from_address(&self) -> String {
        format!("{} <{}>", self.from_name, self.from_email)
    }
}

#[async_trait]
impl EmailProvider for SesProvider {
    async fn send(&self, email: &Email) -> Result<SendResult> {
        let from = email
            .from
            .as_ref()
            .map(|f| f.clone())
            .unwrap_or_else(|| self.format_from_address());

        // Build destination
        let mut destination = Destination::builder().to_addresses(&email.to);

        for cc in &email.cc {
            destination = destination.cc_addresses(cc);
        }
        for bcc in &email.bcc {
            destination = destination.bcc_addresses(bcc);
        }

        // Build message body
        let mut body = Body::builder();

        if let Some(text) = &email.body_text {
            body = body.text(
                Content::builder()
                    .data(text)
                    .charset("UTF-8")
                    .build()
                    .map_err(|e| eyre!("Failed to build text content: {}", e))?,
            );
        }

        if let Some(html) = &email.body_html {
            body = body.html(
                Content::builder()
                    .data(html)
                    .charset("UTF-8")
                    .build()
                    .map_err(|e| eyre!("Failed to build HTML content: {}", e))?,
            );
        }

        let subject = Content::builder()
            .data(&email.subject)
            .charset("UTF-8")
            .build()
            .map_err(|e| eyre!("Failed to build subject: {}", e))?;

        let message = Message::builder()
            .subject(subject)
            .body(body.build())
            .build();

        let email_content = EmailContent::builder().simple(message).build();

        debug!(
            to = %email.to,
            subject = %email.subject,
            "Sending email via AWS SES"
        );

        let mut request = self
            .client
            .send_email()
            .from_email_address(&from)
            .destination(destination.build())
            .content(email_content);

        if let Some(reply_to) = &email.reply_to {
            request = request.reply_to_addresses(reply_to);
        }

        match request.send().await {
            Ok(output) => {
                let message_id = output.message_id().unwrap_or(&email.id).to_string();
                debug!(message_id = %message_id, "Email sent successfully via SES");
                Ok(SendResult { message_id })
            }
            Err(err) => {
                let service_err = err.into_service_error();
                error!(error = %service_err, "AWS SES error");
                Err(eyre!("AWS SES error: {}", service_err))
            }
        }
    }

    async fn health_check(&self) -> Result<()> {
        // Verify the sending identity exists
        self.client
            .get_account()
            .send()
            .await
            .map_err(|e| eyre!("SES health check failed: {}", e.into_service_error()))?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ses"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_address() {
        let provider = SesProvider {
            client: {
                let config = aws_sdk_sesv2::Config::builder()
                    .behavior_version(aws_sdk_sesv2::config::BehaviorVersion::latest())
                    .build();
                aws_sdk_sesv2::Client::from_conf(config)
            },
            from_email: "noreply@example.com".to_string(),
            from_name: "My App".to_string(),
        };

        assert_eq!(
            provider.format_from_address(),
            "My App <noreply@example.com>"
        );
    }
}
