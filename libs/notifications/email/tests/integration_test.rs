//! Integration tests for the email library (NATS-only)

use email::models::{Email, EmailPriority};
use email::provider::{EmailProvider, MockSmtpProvider};
use email::templates::{EmailTemplate, InMemoryTemplateStore, TemplateStore};
use email::{EmailJob, EmailNatsStream, EmailProcessor, EmailType, TemplateEngine};
use messaging::nats::StreamConfig;
use messaging::Processor;
use serde_json::json;

mod template_tests {
    use super::*;

    #[tokio::test]
    async fn test_template_rendering() {
        let store = InMemoryTemplateStore::new();

        // Add a custom template
        let template = EmailTemplate {
            name: "test_template".to_string(),
            subject: "Hello {{name}}!".to_string(),
            body_text: Some("Dear {{name}}, your order #{{order_id}} is ready.".to_string()),
            body_html: Some(
                "<h1>Hello {{name}}!</h1><p>Order #{{order_id}} ready.</p>".to_string(),
            ),
        };

        store.set(template).await.unwrap();

        // Retrieve and render
        let template = store.get("test_template").await.unwrap().unwrap();
        let data = json!({
            "name": "John",
            "order_id": "12345"
        });

        let rendered = template.render(&data).unwrap();

        assert_eq!(rendered.subject, "Hello John!");
        assert_eq!(
            rendered.body_text.unwrap(),
            "Dear John, your order #12345 is ready."
        );
        assert!(rendered.body_html.unwrap().contains("Hello John!"));
    }

    #[tokio::test]
    async fn test_default_templates() {
        let store = InMemoryTemplateStore::with_defaults();

        // Check welcome template exists
        let welcome = store.get("welcome").await.unwrap();
        assert!(welcome.is_some());

        let welcome = welcome.unwrap();
        assert!(welcome.subject.contains("{{app_name}}"));

        // Check password_reset template exists
        let reset = store.get("password_reset").await.unwrap();
        assert!(reset.is_some());
    }

    #[tokio::test]
    async fn test_template_list() {
        let store = InMemoryTemplateStore::new();

        store
            .set(EmailTemplate {
                name: "template1".to_string(),
                subject: "Subject 1".to_string(),
                body_text: None,
                body_html: None,
            })
            .await
            .unwrap();

        store
            .set(EmailTemplate {
                name: "template2".to_string(),
                subject: "Subject 2".to_string(),
                body_text: None,
                body_html: None,
            })
            .await
            .unwrap();

        let templates = store.list().await.unwrap();
        assert_eq!(templates.len(), 2);
        assert!(templates.contains(&"template1".to_string()));
        assert!(templates.contains(&"template2".to_string()));
    }

    #[test]
    fn test_template_engine_defaults() {
        let engine = TemplateEngine::new().unwrap();

        assert!(engine.has_template("welcome"));
        assert!(engine.has_template("verification"));
        assert!(engine.has_template("password_reset"));
        assert!(engine.has_template("password_changed"));
    }

    #[test]
    fn test_template_engine_rendering() {
        let engine = TemplateEngine::new().unwrap();

        let data = json!({
            "name": "Alice",
            "app_name": "TestApp"
        });

        let rendered = engine.render("welcome", &data).unwrap();

        assert!(rendered.subject.contains("Alice"));
        assert!(rendered.subject.contains("TestApp"));
        assert!(rendered.body_text.as_ref().unwrap().contains("Alice"));
    }
}

mod mock_provider_tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_provider_captures_emails() {
        let provider = MockSmtpProvider::new();

        let email1 = Email::new("user1@example.com", "Subject 1").with_text("Body 1");
        let email2 = Email::new("user2@example.com", "Subject 2").with_html("<p>Body 2</p>");

        provider.send(&email1).await.unwrap();
        provider.send(&email2).await.unwrap();

        assert_eq!(provider.sent_count(), 2);

        let sent = provider.sent_emails();
        assert_eq!(sent[0].to, "user1@example.com");
        assert_eq!(sent[1].to, "user2@example.com");
    }

    #[tokio::test]
    async fn test_mock_provider_health_check() {
        let provider = MockSmtpProvider::new();
        assert!(provider.health_check().await.is_ok());

        let failing_provider = MockSmtpProvider::failing("Down for maintenance");
        assert!(failing_provider.health_check().await.is_err());
    }

    #[tokio::test]
    async fn test_mock_provider_clear() {
        let provider = MockSmtpProvider::new();

        let email = Email::new("test@example.com", "Test").with_text("Body");
        provider.send(&email).await.unwrap();

        assert_eq!(provider.sent_count(), 1);

        provider.clear();

        assert_eq!(provider.sent_count(), 0);
    }
}

mod email_model_tests {
    use super::*;

    #[test]
    fn test_email_builder() {
        let email = Email::new("recipient@example.com", "Test Subject")
            .with_text("Plain text body")
            .with_html("<p>HTML body</p>")
            .with_priority(EmailPriority::High)
            .with_template("welcome", json!({"name": "John"}));

        assert_eq!(email.to, "recipient@example.com");
        assert_eq!(email.subject, "Test Subject");
        assert_eq!(email.body_text, Some("Plain text body".to_string()));
        assert_eq!(email.body_html, Some("<p>HTML body</p>".to_string()));
        assert_eq!(email.priority, EmailPriority::High);
        assert_eq!(email.template, Some("welcome".to_string()));
    }

    #[test]
    fn test_email_retry() {
        let mut email = Email::new("test@example.com", "Test");

        assert!(email.can_retry());
        assert_eq!(email.retry_count, 0);

        email.increment_retry();
        assert!(email.can_retry());
        assert_eq!(email.retry_count, 1);

        email.increment_retry();
        email.increment_retry();
        assert!(!email.can_retry()); // Default max_retries is 3
    }

    #[test]
    fn test_email_serialization() {
        let email = Email::new("test@example.com", "Test Subject")
            .with_text("Body")
            .with_priority(EmailPriority::Low);

        let json = serde_json::to_string(&email).unwrap();
        let deserialized: Email = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.to, email.to);
        assert_eq!(deserialized.subject, email.subject);
        assert_eq!(deserialized.priority, email.priority);
    }
}

mod email_job_tests {
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
        assert!(job.to_name.is_some());
        assert_eq!(job.to_name.unwrap(), "John");
    }

    #[test]
    fn test_password_reset_job() {
        let job = EmailJob::password_reset(
            "user@example.com",
            "John",
            "https://example.com/reset?token=abc",
            24,
        );

        assert_eq!(job.email_type, EmailType::PasswordReset);
        assert_eq!(job.priority, EmailPriority::High);
        assert!(job.template_vars.get("reset_link").is_some());
        assert!(job.template_vars.get("expiry_hours").is_some());
    }

    #[test]
    fn test_verification_job() {
        let job = EmailJob::verification(
            "user@example.com",
            "John",
            "https://example.com/verify?token=xyz",
        );

        assert_eq!(job.email_type, EmailType::Verification);
        assert_eq!(job.priority, EmailPriority::High);
        assert!(job.template_vars.get("verification_link").is_some());
    }

    #[test]
    fn test_job_id_survives_redelivery() {
        use messaging::Job;

        // There is no payload-level retry counter: JetStream owns the attempt count
        // (`NatsMessage::delivery_count`). What the job must guarantee is a stable id,
        // so a redelivered message maps to the same DLQ entry and idempotency key.
        let job = EmailJob::new(EmailType::Transactional, "test@example.com", "Test");
        let redelivered: EmailJob =
            serde_json::from_str(&serde_json::to_string(&job).unwrap()).unwrap();

        assert_eq!(redelivered.job_id(), job.job_id());
        assert_eq!(redelivered.job_type(), "email_job");
    }

    #[test]
    fn test_job_serialization() {
        let job = EmailJob::welcome("test@example.com", "Test User", "MyApp");

        let json = serde_json::to_string(&job).unwrap();
        let deserialized: EmailJob = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.to_email, job.to_email);
        assert_eq!(deserialized.email_type, job.email_type);
    }
}

mod processor_tests {
    use super::*;

    #[tokio::test]
    async fn test_processor_with_mock_provider() {
        let provider = MockSmtpProvider::new();
        let templates = TemplateEngine::new().unwrap();
        let processor = EmailProcessor::new(provider, templates);

        // Create a job with body (no template)
        let job = EmailJob::new(EmailType::Transactional, "test@example.com", "Test Subject")
            .with_text("Test body");

        // Process should succeed
        let result = processor.process(&job).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_processor_name() {
        let provider = MockSmtpProvider::new();
        let templates = TemplateEngine::new().unwrap();
        let processor = EmailProcessor::new(provider, templates);

        assert_eq!(
            messaging::Processor::<EmailJob>::name(&processor),
            "email_processor"
        );
    }

    #[tokio::test]
    async fn test_processor_health_check() {
        let provider = MockSmtpProvider::new();
        let templates = TemplateEngine::new().unwrap();
        let processor = EmailProcessor::new(provider, templates);

        let result = processor.health_check().await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }
}

mod stream_config_tests {
    use super::*;

    #[test]
    fn test_email_nats_stream_config() {
        assert_eq!(EmailNatsStream::STREAM_NAME, "EMAILS");
        assert_eq!(EmailNatsStream::CONSUMER_NAME, "email-worker");
        assert_eq!(EmailNatsStream::DLQ_STREAM, "EMAILS_DLQ");
        assert_eq!(EmailNatsStream::SUBJECT, "emails.>");
        assert_eq!(EmailNatsStream::MAX_DELIVER, 5);
        assert_eq!(EmailNatsStream::ACK_WAIT_SECS, 30);
    }
}

// NATS integration tests (requires Docker)
#[cfg(feature = "integration")]
mod nats_integration_tests {
    use super::*;
    use test_utils::TestNats;

    #[tokio::test]
    async fn test_email_job_publish_consume() {
        let nats = TestNats::new().await;
        let jetstream = nats.jetstream();

        // Create stream
        let stream_config = async_nats::jetstream::stream::Config {
            name: EmailNatsStream::STREAM_NAME.to_string(),
            subjects: vec![EmailNatsStream::SUBJECT.to_string()],
            ..Default::default()
        };

        jetstream.create_stream(stream_config).await.unwrap();

        // Publish a job
        let job = EmailJob::welcome("test@example.com", "Test User", "MyApp");
        let payload = serde_json::to_vec(&job).unwrap();

        let ack = jetstream
            .publish("emails.welcome", payload.into())
            .await
            .unwrap()
            .await
            .unwrap();

        assert!(ack.sequence > 0);

        // Verify stream has the message
        let mut stream = jetstream
            .get_stream(EmailNatsStream::STREAM_NAME)
            .await
            .unwrap();
        let info = stream.info().await.unwrap();
        assert_eq!(info.state.messages, 1);
    }
}
