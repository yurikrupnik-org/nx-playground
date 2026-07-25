//! Mock email provider for testing

use super::{EmailProvider, ProviderError, SendResult};
use crate::models::Email;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Mock email provider that captures sent emails
#[derive(Default)]
pub struct MockSmtpProvider {
    sent_emails: Mutex<Vec<Email>>,
    should_fail: AtomicBool,
    failure_message: Option<String>,
}

impl MockSmtpProvider {
    /// Create a new mock provider
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a mock provider that always fails
    pub fn failing(message: impl Into<String>) -> Self {
        Self {
            sent_emails: Mutex::new(Vec::new()),
            should_fail: AtomicBool::new(true),
            failure_message: Some(message.into()),
        }
    }

    /// Toggle failure mode at runtime.
    pub fn set_should_fail(&self, should_fail: bool) {
        self.should_fail.store(should_fail, Ordering::Relaxed);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Email>> {
        self.sent_emails
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Get all sent emails
    pub fn sent_emails(&self) -> Vec<Email> {
        self.lock().clone()
    }

    /// Get the count of sent emails
    pub fn sent_count(&self) -> usize {
        self.lock().len()
    }

    /// Clear all sent emails
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Check if an email was sent to a specific address
    pub fn was_sent_to(&self, email: &str) -> bool {
        self.lock().iter().any(|e| e.to == email)
    }
}

#[async_trait]
impl EmailProvider for MockSmtpProvider {
    async fn send(&self, email: &Email) -> Result<SendResult, ProviderError> {
        if self.should_fail.load(Ordering::Relaxed) {
            let message = self.failure_message.as_deref().unwrap_or("Mock failure");
            return Err(ProviderError::transient(message));
        }

        self.lock().push(email.clone());

        Ok(SendResult {
            message_id: format!("mock-{}", email.id),
        })
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        if self.should_fail.load(Ordering::Relaxed) {
            return Err(ProviderError::transient("Mock health check failed"));
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_provider_sends_email() {
        let provider = MockSmtpProvider::new();

        let email = Email::new("test@example.com", "Test Subject").with_text("Test body");

        let result = provider.send(&email).await;
        assert!(result.is_ok());

        let sent = provider.sent_emails();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].to, "test@example.com");
    }

    #[tokio::test]
    async fn test_mock_provider_fails() {
        let provider = MockSmtpProvider::failing("Simulated failure");

        let email = Email::new("test@example.com", "Test Subject").with_text("Test body");

        let result = provider.send(&email).await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Simulated failure"));
    }

    #[tokio::test]
    async fn test_mock_provider_toggle_failure() {
        let provider = MockSmtpProvider::new();
        let email = Email::new("test@example.com", "Test").with_text("Body");

        provider.set_should_fail(true);
        assert!(provider.send(&email).await.is_err());

        provider.set_should_fail(false);
        assert!(provider.send(&email).await.is_ok());
        assert_eq!(provider.sent_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_provider_was_sent_to() {
        let provider = MockSmtpProvider::new();

        let email = Email::new("user@example.com", "Test").with_text("Body");
        provider.send(&email).await.unwrap();

        assert!(provider.was_sent_to("user@example.com"));
        assert!(!provider.was_sent_to("other@example.com"));
    }
}
