//! EmailProcessor - Implements processor for NATS backend
//!
//! This module provides the processor that handles EmailJob processing.
//! Implements `messaging::Processor` for NATS JetStream.
//!
//! IMPROVEMENT: Removed Redis StreamProcessor - this is now NATS-only.

use crate::Email;
use crate::job::{EmailJob, EmailType};
use crate::provider::{EmailProvider, SendResult};
use crate::templates::TemplateEngine;
use messaging::ProcessingError;
use std::sync::Arc;
use tracing::{debug, info};

/// Email processor that sends emails using a provider
pub struct EmailProcessor<P: EmailProvider> {
    provider: Arc<P>,
    templates: Arc<TemplateEngine>,
    from_email: String,
    from_name: String,
}

impl<P: EmailProvider> EmailProcessor<P> {
    /// Create a new EmailProcessor
    pub fn new(provider: P, templates: TemplateEngine) -> Self {
        Self {
            provider: Arc::new(provider),
            templates: Arc::new(templates),
            from_email: std::env::var("EMAIL_FROM_ADDRESS")
                .unwrap_or_else(|_| "noreply@example.com".to_string()),
            from_name: std::env::var("EMAIL_FROM_NAME")
                .unwrap_or_else(|_| "Notifications".to_string()),
        }
    }

    /// Create with explicit from address
    pub fn with_from(mut self, email: impl Into<String>, name: impl Into<String>) -> Self {
        self.from_email = email.into();
        self.from_name = name.into();
        self
    }

    /// Get the template name for an email type
    fn template_name(email_type: &EmailType) -> Option<&'static str> {
        match email_type {
            EmailType::Welcome => Some("welcome"),
            EmailType::Verification => Some("verification"),
            EmailType::PasswordReset => Some("password_reset"),
            EmailType::PasswordChanged => Some("password_changed"),
            EmailType::TaskNotification => Some("task_notification"),
            EmailType::Transactional => None,
            EmailType::Custom(name) => {
                // For custom templates, we'd need to store and return the name
                // For now, return None and let the job use body_text/body_html
                debug!(template = %name, "Custom template requested");
                None
            }
        }
    }

    /// Render an email job into a sendable Email
    fn render_job(&self, job: &EmailJob) -> Result<Email, ProcessingError> {
        let template_name = Self::template_name(&job.email_type);

        let (subject, body_text, body_html) = if let Some(name) = template_name {
            // Render template
            let rendered = self
                .templates
                .render(name, &job.template_vars)
                .map_err(|e| ProcessingError::permanent(format!("Template error: {e}")))?;

            (rendered.subject, rendered.body_text, rendered.body_html)
        } else {
            // Use direct body from job
            (
                job.subject.clone(),
                job.body_text.clone(),
                job.body_html.clone(),
            )
        };

        // Ensure we have at least text or HTML
        if body_text.is_none() && body_html.is_none() {
            return Err(ProcessingError::permanent(
                "Email must have either text or HTML body",
            ));
        }

        let mut email = Email::new(&job.to_email, subject);
        email.from = Some(format!("{} <{}>", self.from_name, self.from_email));

        if let Some(text) = body_text {
            email.body_text = Some(text);
        }
        if let Some(html) = body_html {
            email.body_html = Some(html);
        }

        email.priority = job.priority.clone();

        Ok(email)
    }

    /// Send an email and handle the result
    async fn send_email(&self, email: &Email) -> Result<SendResult, ProcessingError> {
        self.provider
            .send(email)
            .await
            .map_err(ProcessingError::from)
    }
}

impl<P: EmailProvider + 'static> messaging::Processor<EmailJob> for EmailProcessor<P> {
    async fn process(&self, job: &EmailJob) -> Result<(), ProcessingError> {
        debug!(
            job_id = %job.id,
            email_type = ?job.email_type,
            to = %job.to_email,
            "Processing email job"
        );

        // Render the email
        let email = self.render_job(job)?;

        // Send it
        let result = self.send_email(&email).await?;

        info!(
            job_id = %job.id,
            message_id = %result.message_id,
            to = %job.to_email,
            "Email sent successfully"
        );

        Ok(())
    }

    fn name(&self) -> &'static str {
        "email_processor"
    }

    async fn health_check(&self) -> Result<bool, ProcessingError> {
        self.provider
            .health_check()
            .await
            .map(|_| true)
            .map_err(|e| ProcessingError::transient(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockSmtpProvider;

    #[tokio::test]
    async fn test_processor_creation() {
        let provider = MockSmtpProvider::new();
        let templates = TemplateEngine::new().unwrap();
        let processor = EmailProcessor::new(provider, templates);

        assert_eq!(
            messaging::Processor::<EmailJob>::name(&processor),
            "email_processor"
        );
    }
}
