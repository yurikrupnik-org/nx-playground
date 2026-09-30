//! EmailJob - Implements Job traits for email processing
//!
//! This module provides the job type used by NATS JetStream workers.
//!
//! IMPROVEMENT: Removed StreamJob implementation - this is now NATS-only.

use crate::models::{Email, EmailPriority};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Re-export Job trait for convenience
pub use messaging::Job as MessagingJob;

/// Email type variants for different email templates
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EmailType {
    /// Welcome email for new users
    Welcome,
    /// Email verification
    Verification,
    /// Password reset request
    PasswordReset,
    /// Password changed confirmation
    PasswordChanged,
    /// Task-related notifications (assigned, due soon, overdue, completed)
    TaskNotification,
    /// Generic transactional email
    #[default]
    Transactional,
    /// Custom template
    Custom(String),
}

impl EmailType {
    /// Get the NATS subject suffix for this email type.
    ///
    /// Used for routing emails to specific subjects (e.g., "emails.welcome").
    pub fn subject_suffix(&self) -> &str {
        match self {
            EmailType::Welcome => "welcome",
            EmailType::Verification => "verification",
            EmailType::PasswordReset => "password_reset",
            EmailType::PasswordChanged => "password_changed",
            EmailType::TaskNotification => "task",
            EmailType::Transactional => "transactional",
            EmailType::Custom(_) => "custom",
        }
    }
}

/// Email job for stream processing
///
/// This struct implements `messaging::Job` and contains all data needed
/// to process an email through the NATS worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailJob {
    /// Unique job ID
    pub id: Uuid,

    /// Type of email (determines template)
    pub email_type: EmailType,

    /// Recipient email address
    pub to_email: String,

    /// Optional recipient name
    pub to_name: Option<String>,

    /// Email subject (can be overridden by template)
    pub subject: String,

    /// Template variables
    #[serde(default)]
    pub template_vars: serde_json::Value,

    /// Plain text body (for non-template emails)
    pub body_text: Option<String>,

    /// HTML body (for non-template emails)
    pub body_html: Option<String>,

    /// Email priority
    #[serde(default)]
    pub priority: EmailPriority,

    /// When the job was created
    pub created_at: DateTime<Utc>,
}

impl EmailJob {
    /// Create a new EmailJob
    pub fn new(
        email_type: EmailType,
        to_email: impl Into<String>,
        subject: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            email_type,
            to_email: to_email.into(),
            to_name: None,
            subject: subject.into(),
            template_vars: serde_json::Value::Null,
            body_text: None,
            body_html: None,
            priority: EmailPriority::Normal,
            created_at: Utc::now(),
        }
    }

    /// Create from an existing Email
    pub fn from_email(email: &Email) -> Self {
        let email_type = email
            .template
            .as_ref()
            .map(|t| match t.as_str() {
                "welcome" => EmailType::Welcome,
                "verification" => EmailType::Verification,
                "password_reset" => EmailType::PasswordReset,
                "password_changed" => EmailType::PasswordChanged,
                other => EmailType::Custom(other.to_string()),
            })
            .unwrap_or(EmailType::Transactional);

        Self {
            id: Uuid::parse_str(&email.id).unwrap_or_else(|_| Uuid::new_v4()),
            email_type,
            to_email: email.to.clone(),
            to_name: None,
            subject: email.subject.clone(),
            template_vars: email.template_data.clone(),
            body_text: email.body_text.clone(),
            body_html: email.body_html.clone(),
            priority: email.priority.clone(),
            created_at: Utc::now(),
        }
    }

    /// Set recipient name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.to_name = Some(name.into());
        self
    }

    /// Set template variables
    pub fn with_vars(mut self, vars: serde_json::Value) -> Self {
        self.template_vars = vars;
        self
    }

    /// Set plain text body
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.body_text = Some(text.into());
        self
    }

    /// Set HTML body
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.body_html = Some(html.into());
        self
    }

    /// Set priority
    pub fn with_priority(mut self, priority: EmailPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Create a welcome email job
    pub fn welcome(
        to_email: impl Into<String>,
        name: impl Into<String>,
        app_name: impl Into<String>,
    ) -> Self {
        let name = name.into();
        Self::new(
            EmailType::Welcome,
            to_email,
            format!("Welcome to {}", app_name.into()),
        )
        .with_name(name.clone())
        .with_vars(serde_json::json!({
            "name": name,
        }))
    }

    /// Create a password reset email job
    pub fn password_reset(
        to_email: impl Into<String>,
        name: impl Into<String>,
        reset_link: impl Into<String>,
        expiry_hours: u32,
    ) -> Self {
        let name = name.into();
        Self::new(EmailType::PasswordReset, to_email, "Password Reset Request")
            .with_name(name.clone())
            .with_priority(EmailPriority::High)
            .with_vars(serde_json::json!({
                "name": name,
                "reset_link": reset_link.into(),
                "expiry_hours": expiry_hours,
            }))
    }

    /// Create a verification email job
    pub fn verification(
        to_email: impl Into<String>,
        name: impl Into<String>,
        verification_link: impl Into<String>,
    ) -> Self {
        let name = name.into();
        Self::new(EmailType::Verification, to_email, "Verify Your Email")
            .with_name(name.clone())
            .with_priority(EmailPriority::High)
            .with_vars(serde_json::json!({
                "name": name,
                "verification_link": verification_link.into(),
            }))
    }
}

// Implement messaging::Job for NATS backend.
//
// No retry surface: JetStream owns the attempt count (`NatsMessage::delivery_count`)
// and the per-category policy in `messaging::ErrorCategory` owns the ceiling. The
// old `max_retries()` override, which varied the cap by EmailPriority, never took
// effect — it was only reachable through `can_retry()`, which nothing called.
impl MessagingJob for EmailJob {
    fn job_id(&self) -> Uuid {
        self.id
    }

    fn job_type(&self) -> &'static str {
        "email_job"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_job_creation() {
        let job = EmailJob::new(EmailType::Welcome, "test@example.com", "Welcome!");

        assert_eq!(job.to_email, "test@example.com");
        assert_eq!(job.subject, "Welcome!");
        assert_eq!(job.email_type, EmailType::Welcome);
        assert_eq!(job.priority, EmailPriority::Normal);
    }

    #[test]
    fn test_welcome_job() {
        let job = EmailJob::welcome("user@example.com", "John", "MyApp");

        assert_eq!(job.email_type, EmailType::Welcome);
        assert_eq!(job.to_email, "user@example.com");
        assert!(job.subject.contains("MyApp"));
    }

    #[test]
    fn test_messaging_job_impl() {
        use messaging::Job;

        let job = EmailJob::new(EmailType::Transactional, "test@example.com", "Test");

        assert!(!Job::job_id(&job).is_nil());
        assert_eq!(Job::job_id(&job), job.id);
        assert_eq!(Job::job_type(&job), "email_job");
        // The id must survive a redelivery so DLQ entries and idempotency keys line up.
        assert_eq!(Job::job_id(&job.clone()), Job::job_id(&job));
    }
}
