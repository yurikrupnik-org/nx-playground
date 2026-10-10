//! Notification service for queueing email jobs via NATS JetStream.
//!
//! This service provides a high-level API for queueing emails to be processed
//! by the email worker. It uses `NatsProducer` from the messaging library.

use crate::error::{NotificationError, NotificationResult};
use crate::job::{EmailJob, EmailType};
use crate::streams::EmailNatsStream;
use messaging::nats::{NatsProducer, StreamConfig};
use serde::Serialize;
use serde_json::json;
use tracing::{debug, info};
use uuid::Uuid;

/// Configuration for the notification service.
#[derive(Debug, Clone)]
pub struct NotificationServiceConfig {
    /// Base URL for the frontend application.
    pub frontend_url: String,
    /// Email verification token expiry in hours.
    pub verification_expiry_hours: i64,
    /// Password reset token expiry in hours.
    pub password_reset_expiry_hours: i64,
    /// Company name for email footers.
    pub company_name: String,
    /// Company address for email footers.
    pub company_address: String,
    /// Logo URL for emails.
    pub logo_url: String,
}

impl Default for NotificationServiceConfig {
    fn default() -> Self {
        Self {
            frontend_url: std::env::var("FRONTEND_URL")
                .unwrap_or_else(|_| "http://localhost:3000".to_string()),
            verification_expiry_hours: std::env::var("EMAIL_VERIFICATION_EXPIRY_HOURS")
                .unwrap_or_else(|_| "24".to_string())
                .parse()
                .unwrap_or(24),
            password_reset_expiry_hours: std::env::var("PASSWORD_RESET_EXPIRY_HOURS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()
                .unwrap_or(1),
            company_name: std::env::var("COMPANY_NAME").unwrap_or_else(|_| "Zerg".to_string()),
            company_address: std::env::var("COMPANY_ADDRESS").unwrap_or_else(|_| String::new()),
            logo_url: std::env::var("LOGO_URL").unwrap_or_else(|_| String::new()),
        }
    }
}

/// Data for welcome email templates.
#[derive(Debug, Clone, Serialize)]
pub struct WelcomeEmailData {
    pub user_name: String,
    pub user_email: String,
    pub requires_verification: bool,
    pub verification_url: Option<String>,
    pub verification_expiry_hours: u32,
    pub dashboard_url: String,
    pub account_type: String,
    pub join_date: String,
    pub logo_url: String,
    pub preferences_url: String,
    pub help_url: String,
    pub privacy_url: String,
    pub company_name: String,
    pub company_address: String,
}

/// Service for managing email notifications via NATS JetStream.
///
/// This service wraps `NatsProducer` and provides high-level methods
/// for queueing common email types (welcome, verification, password reset, etc.).
#[derive(Clone)]
pub struct NotificationService {
    producer: NatsProducer,
    config: NotificationServiceConfig,
}

impl NotificationService {
    /// Create a new notification service.
    pub fn new(producer: NatsProducer, config: NotificationServiceConfig) -> Self {
        Self { producer, config }
    }

    /// Create a notification service with the default config.
    pub fn with_default_config(producer: NatsProducer) -> Self {
        Self::new(producer, NotificationServiceConfig::default())
    }

    /// Create a notification service from a JetStream context.
    pub fn from_jetstream(
        jetstream: async_nats::jetstream::Context,
        config: NotificationServiceConfig,
    ) -> Self {
        let producer = NatsProducer::from_stream_config::<EmailNatsStream>(jetstream);
        Self::new(producer, config)
    }

    /// Create a notification service from a JetStream context with default config.
    pub fn from_jetstream_default(jetstream: async_nats::jetstream::Context) -> Self {
        Self::from_jetstream(jetstream, NotificationServiceConfig::default())
    }

    /// Generate a secure random token (64 alphanumeric characters).
    pub fn generate_token() -> String {
        use rand::RngExt;
        use std::iter;
        let mut rng = rand::rng();
        iter::repeat_with(|| {
            let idx = rng.random_range(0..62);
            match idx {
                0..=9 => (b'0' + idx) as char,
                10..=35 => (b'a' + idx - 10) as char,
                _ => (b'A' + idx - 36) as char,
            }
        })
        .take(64)
        .collect()
    }

    /// Get the stream name.
    pub fn stream_name(&self) -> &str {
        EmailNatsStream::STREAM_NAME
    }

    /// Get the subject.
    pub fn subject(&self) -> &str {
        EmailNatsStream::SUBJECT
    }

    /// Queue an email job to NATS JetStream.
    ///
    /// Returns the sequence number of the published message.
    async fn queue_job(&self, job: &EmailJob) -> NotificationResult<u64> {
        // Use a specific subject for the email type
        let subject = format!("emails.{}", job.email_type.subject_suffix());

        self.producer
            .send_to(&subject, job)
            .await
            .map_err(NotificationError::Queue)
    }

    /// Queue a welcome email for a new user.
    ///
    /// This sends a combined welcome + verification email if the user
    /// needs to verify their email address.
    pub async fn queue_welcome_email(
        &self,
        user_id: Uuid,
        email: &str,
        name: &str,
        requires_verification: bool,
        verification_token: Option<&str>,
    ) -> NotificationResult<u64> {
        let verification_url = verification_token.map(|token| {
            format!(
                "{}/auth/verify-email?token={}",
                self.config.frontend_url, token
            )
        });

        let template_data = WelcomeEmailData {
            user_name: name.to_string(),
            user_email: email.to_string(),
            requires_verification,
            verification_url,
            verification_expiry_hours: self.config.verification_expiry_hours as u32,
            dashboard_url: format!("{}/dashboard", self.config.frontend_url),
            account_type: "Free".to_string(),
            join_date: chrono::Utc::now().format("%B %d, %Y").to_string(),
            logo_url: self.config.logo_url.clone(),
            preferences_url: format!("{}/settings/notifications", self.config.frontend_url),
            help_url: format!("{}/help", self.config.frontend_url),
            privacy_url: format!("{}/privacy", self.config.frontend_url),
            company_name: self.config.company_name.clone(),
            company_address: self.config.company_address.clone(),
        };

        let job = EmailJob::new(
            EmailType::Welcome,
            email.to_string(),
            format!("Welcome to {}, {}!", self.config.company_name, name),
        )
        .with_name(name)
        .with_vars(serde_json::to_value(&template_data)?);

        let sequence = self.queue_job(&job).await?;

        info!(
            user_id = %user_id,
            email = %email,
            sequence = sequence,
            requires_verification = %requires_verification,
            "Queued welcome email to NATS"
        );

        Ok(sequence)
    }

    /// Queue a standalone verification email (for resend requests).
    pub async fn queue_verification_email(
        &self,
        user_id: Uuid,
        email: &str,
        name: &str,
        verification_token: &str,
    ) -> NotificationResult<u64> {
        let verification_url = format!(
            "{}/auth/verify-email?token={}",
            self.config.frontend_url, verification_token
        );

        let template_data = json!({
            "user_name": name,
            "verification_url": verification_url,
            "verification_expiry_hours": self.config.verification_expiry_hours,
            "logo_url": self.config.logo_url,
            "help_url": format!("{}/help", self.config.frontend_url),
            "company_name": self.config.company_name,
        });

        let job = EmailJob::new(
            EmailType::Verification,
            email.to_string(),
            "Verify your email address",
        )
        .with_name(name)
        .with_vars(template_data);

        let sequence = self.queue_job(&job).await?;

        info!(
            user_id = %user_id,
            email = %email,
            sequence = sequence,
            "Queued verification email to NATS"
        );

        Ok(sequence)
    }

    /// Queue a password reset email.
    pub async fn queue_password_reset_email(
        &self,
        user_id: Uuid,
        email: &str,
        name: &str,
        reset_token: &str,
    ) -> NotificationResult<u64> {
        let reset_url = format!(
            "{}/auth/reset-password?token={}",
            self.config.frontend_url, reset_token
        );

        let job = EmailJob::password_reset(
            email,
            name,
            &reset_url,
            self.config.password_reset_expiry_hours as u32,
        );

        let sequence = self.queue_job(&job).await?;

        info!(
            user_id = %user_id,
            email = %email,
            sequence = sequence,
            "Queued password reset email to NATS"
        );

        Ok(sequence)
    }

    /// Queue a task notification email.
    #[allow(clippy::too_many_arguments)]
    pub async fn queue_task_notification(
        &self,
        user_id: Uuid,
        email: &str,
        name: &str,
        notification_type: &str,
        task_title: &str,
        task_description: Option<&str>,
        task_url: &str,
        due_date: Option<&str>,
        assigned_by: Option<&str>,
    ) -> NotificationResult<u64> {
        let template_data = json!({
            "user_name": name,
            "notification_type": notification_type,
            "task_title": task_title,
            "task_description": task_description,
            "task_url": task_url,
            "due_date": due_date,
            "assigned_by": assigned_by,
            "logo_url": self.config.logo_url,
            "preferences_url": format!("{}/settings/notifications", self.config.frontend_url),
            "company_name": self.config.company_name,
        });

        let subject = match notification_type {
            "assigned" => format!("Task assigned: {task_title}"),
            "due_soon" => format!("Task due soon: {task_title}"),
            "overdue" => format!("Task overdue: {task_title}"),
            "completed" => format!("Task completed: {task_title}"),
            _ => format!("Task update: {task_title}"),
        };

        let job = EmailJob::new(EmailType::TaskNotification, email.to_string(), subject)
            .with_name(name)
            .with_vars(template_data);

        let sequence = self.queue_job(&job).await?;

        info!(
            user_id = %user_id,
            email = %email,
            sequence = sequence,
            notification_type = %notification_type,
            task = %task_title,
            "Queued task notification email to NATS"
        );

        Ok(sequence)
    }

    /// Queue a generic email job.
    pub async fn queue_email(&self, job: EmailJob) -> NotificationResult<u64> {
        let sequence = self.queue_job(&job).await?;

        debug!(
            job_id = %job.id,
            sequence = sequence,
            email_type = ?job.email_type,
            to = %job.to_email,
            "Queued email job to NATS"
        );

        Ok(sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token() {
        let token = NotificationService::generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_alphanumeric()));
    }

    #[test]
    fn test_default_config() {
        let config = NotificationServiceConfig::default();
        assert_eq!(config.verification_expiry_hours, 24);
        assert_eq!(config.password_reset_expiry_hours, 1);
        assert_eq!(config.company_name, "Zerg");
    }
}
