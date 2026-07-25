//! SMTP email provider using lettre

use super::{EmailProvider, ProviderError, SendResult};
use crate::error::{NotificationError, NotificationResult};
use crate::models::Email;
use async_trait::async_trait;
use lettre::{
    message::{header::ContentType, Mailbox, MultiPart, SinglePart},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};

/// SMTP provider configuration
#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from_email: String,
    pub from_name: String,
    pub use_tls: bool,
}

/// SMTP email provider
pub struct SmtpProvider {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    /// From mailbox, parsed once at construction.
    from: Mailbox,
}

/// Classify a lettre SMTP transport error by its own response classification.
fn classify_smtp_error(e: lettre::transport::smtp::Error) -> ProviderError {
    // 5xx SMTP replies and client-side misuse will not succeed on retry;
    // 4xx replies, network/timeout/TLS failures may.
    if e.is_permanent() || e.is_client() {
        ProviderError::permanent_with_source("SMTP send rejected", e)
    } else {
        ProviderError::transient_with_source("SMTP send failed", e)
    }
}

impl SmtpProvider {
    /// Create a new SMTP provider
    pub fn new(config: SmtpConfig) -> NotificationResult<Self> {
        let from: Mailbox = format!("{} <{}>", config.from_name, config.from_email)
            .parse()
            .map_err(|e| NotificationError::Config(format!("invalid from address: {e}")))?;

        let transport = if config.use_tls {
            let creds = Credentials::new(config.username, config.password);
            AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(|e| {
                    NotificationError::Config(format!("failed to create SMTP relay: {e}"))
                })?
                .credentials(creds)
                .port(config.port)
                .build()
        } else if !config.username.is_empty() {
            let creds = Credentials::new(config.username, config.password);
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
                .credentials(creds)
                .port(config.port)
                .build()
        } else {
            // No auth (for Mailpit/Mailhog)
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
                .port(config.port)
                .build()
        };

        Ok(Self { transport, from })
    }

    /// Create a provider for Mailhog/Mailpit (local development)
    ///
    /// Connects to localhost:1025 without authentication.
    pub fn mailhog() -> NotificationResult<Self> {
        let host = std::env::var("SMTP_HOST").unwrap_or_else(|_| "localhost".to_string());
        let port: u16 = std::env::var("SMTP_PORT")
            .unwrap_or_else(|_| "1025".to_string())
            .parse()
            .map_err(|e| NotificationError::Config(format!("invalid SMTP_PORT: {e}")))?;

        let config = SmtpConfig {
            host,
            port,
            username: String::new(),
            password: String::new(),
            from_email: std::env::var("EMAIL_FROM_ADDRESS")
                .unwrap_or_else(|_| "noreply@localhost".to_string()),
            from_name: std::env::var("EMAIL_FROM_NAME")
                .unwrap_or_else(|_| "Development".to_string()),
            use_tls: false,
        };

        Self::new(config)
    }

    /// Create a provider from environment variables
    pub fn from_env() -> NotificationResult<Self> {
        let config = SmtpConfig {
            host: std::env::var("SMTP_HOST")
                .map_err(|_| NotificationError::Config("SMTP_HOST not set".into()))?,
            port: std::env::var("SMTP_PORT")
                .unwrap_or_else(|_| "587".to_string())
                .parse()
                .map_err(|e| NotificationError::Config(format!("invalid SMTP_PORT: {e}")))?,
            username: std::env::var("SMTP_USERNAME").unwrap_or_default(),
            password: std::env::var("SMTP_PASSWORD").unwrap_or_default(),
            from_email: std::env::var("EMAIL_FROM_ADDRESS")
                .or_else(|_| std::env::var("SMTP_FROM_EMAIL"))
                .map_err(|_| NotificationError::Config("EMAIL_FROM_ADDRESS not set".into()))?,
            from_name: std::env::var("EMAIL_FROM_NAME")
                .unwrap_or_else(|_| "Notifications".to_string()),
            use_tls: std::env::var("SMTP_USE_TLS")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(true),
        };

        Self::new(config)
    }

    /// Create a provider for Gmail with App Password
    ///
    /// Uses `smtp.gmail.com:587` with STARTTLS.
    ///
    /// # Setup
    /// 1. Enable 2-Factor Authentication on your Google account
    /// 2. Generate an App Password at https://myaccount.google.com/apppasswords
    /// 3. Set environment variables:
    ///    - `GMAIL_USER` - Your Gmail address
    ///    - `GMAIL_APP_PASSWORD` - The 16-character app password
    ///    - `EMAIL_FROM_ADDRESS` or `GMAIL_FROM_EMAIL` (optional, defaults to GMAIL_USER)
    ///    - `EMAIL_FROM_NAME` (optional)
    pub fn gmail_app_password() -> NotificationResult<Self> {
        let username = std::env::var("GMAIL_USER")
            .map_err(|_| NotificationError::Config("GMAIL_USER not set".into()))?;
        let password = std::env::var("GMAIL_APP_PASSWORD")
            .map_err(|_| NotificationError::Config("GMAIL_APP_PASSWORD not set".into()))?;

        let from_email = std::env::var("GMAIL_FROM_EMAIL")
            .or_else(|_| std::env::var("EMAIL_FROM_ADDRESS"))
            .unwrap_or_else(|_| username.clone());

        let config = SmtpConfig {
            host: "smtp.gmail.com".to_string(),
            port: 587,
            username,
            password,
            from_email,
            from_name: std::env::var("EMAIL_FROM_NAME")
                .unwrap_or_else(|_| "Notifications".to_string()),
            use_tls: true,
        };

        Self::new(config)
    }

    /// Create a provider for Google Workspace SMTP Relay
    ///
    /// Uses `smtp-relay.gmail.com:587` with STARTTLS.
    ///
    /// # Setup (IP-based authentication)
    /// 1. In Google Admin Console, go to Apps > Google Workspace > Gmail > Routing
    /// 2. Add your server's IP to the SMTP relay service
    /// 3. Set environment variables:
    ///    - `EMAIL_FROM_ADDRESS` - Must be from your Workspace domain
    ///    - `EMAIL_FROM_NAME` (optional)
    pub fn gmail_relay() -> NotificationResult<Self> {
        let from_email = std::env::var("EMAIL_FROM_ADDRESS")
            .or_else(|_| std::env::var("GMAIL_FROM_EMAIL"))
            .map_err(|_| NotificationError::Config("EMAIL_FROM_ADDRESS not set".into()))?;

        let config = SmtpConfig {
            host: "smtp-relay.gmail.com".to_string(),
            port: 587,
            username: String::new(), // No auth for IP-allowlisted relay
            password: String::new(),
            from_email,
            from_name: std::env::var("EMAIL_FROM_NAME")
                .unwrap_or_else(|_| "Notifications".to_string()),
            use_tls: true,
        };

        Self::new(config)
    }

    /// Create a provider for Google Workspace SMTP Relay with credentials
    ///
    /// Uses `smtp-relay.gmail.com:587` with SMTP AUTH.
    pub fn gmail_relay_with_auth() -> NotificationResult<Self> {
        let username = std::env::var("GMAIL_RELAY_USER")
            .map_err(|_| NotificationError::Config("GMAIL_RELAY_USER not set".into()))?;
        let password = std::env::var("GMAIL_RELAY_PASSWORD")
            .map_err(|_| NotificationError::Config("GMAIL_RELAY_PASSWORD not set".into()))?;

        let from_email = std::env::var("EMAIL_FROM_ADDRESS")
            .or_else(|_| std::env::var("GMAIL_FROM_EMAIL"))
            .map_err(|_| NotificationError::Config("EMAIL_FROM_ADDRESS not set".into()))?;

        let config = SmtpConfig {
            host: "smtp-relay.gmail.com".to_string(),
            port: 587,
            username,
            password,
            from_email,
            from_name: std::env::var("EMAIL_FROM_NAME")
                .unwrap_or_else(|_| "Notifications".to_string()),
            use_tls: true,
        };

        Self::new(config)
    }

    fn build_message(&self, email: &Email) -> Result<Message, ProviderError> {
        let to: Mailbox = email
            .to
            .parse()
            .map_err(|e| ProviderError::permanent_with_source("Invalid to address", e))?;

        let mut builder = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(&email.subject);

        // Add reply-to if specified
        if let Some(reply_to) = &email.reply_to {
            let reply_to_mailbox: Mailbox = reply_to
                .parse()
                .map_err(|e| ProviderError::permanent_with_source("Invalid reply-to address", e))?;
            builder = builder.reply_to(reply_to_mailbox);
        }

        // Add CC recipients
        for cc in &email.cc {
            let cc_mailbox: Mailbox = cc
                .parse()
                .map_err(|e| ProviderError::permanent_with_source("Invalid CC address", e))?;
            builder = builder.cc(cc_mailbox);
        }

        // Add BCC recipients
        for bcc in &email.bcc {
            let bcc_mailbox: Mailbox = bcc
                .parse()
                .map_err(|e| ProviderError::permanent_with_source("Invalid BCC address", e))?;
            builder = builder.bcc(bcc_mailbox);
        }

        // Build body.
        // lettre's `Body` is owned, so the text/html payloads are necessarily
        // copied out of the borrowed `Email` here.
        let message = match (&email.body_text, &email.body_html) {
            (Some(text), Some(html)) => builder
                .multipart(
                    MultiPart::alternative()
                        .singlepart(
                            SinglePart::builder()
                                .header(ContentType::TEXT_PLAIN)
                                .body(text.clone()),
                        )
                        .singlepart(
                            SinglePart::builder()
                                .header(ContentType::TEXT_HTML)
                                .body(html.clone()),
                        ),
                )
                .map_err(|e| {
                    ProviderError::permanent_with_source("Failed to build multipart message", e)
                })?,
            (Some(text), None) => builder
                .header(ContentType::TEXT_PLAIN)
                .body(text.clone())
                .map_err(|e| {
                    ProviderError::permanent_with_source("Failed to build text message", e)
                })?,
            (None, Some(html)) => builder
                .header(ContentType::TEXT_HTML)
                .body(html.clone())
                .map_err(|e| {
                    ProviderError::permanent_with_source("Failed to build HTML message", e)
                })?,
            (None, None) => {
                return Err(ProviderError::permanent(
                    "Email must have either text or HTML body",
                ));
            }
        };

        Ok(message)
    }
}

#[async_trait]
impl EmailProvider for SmtpProvider {
    async fn send(&self, email: &Email) -> Result<SendResult, ProviderError> {
        let message = self.build_message(email)?;

        let response = self
            .transport
            .send(message)
            .await
            .map_err(classify_smtp_error)?;

        // Extract message ID from response
        let message_id = response
            .message()
            .next()
            .map(|s| s.to_string())
            .unwrap_or_else(|| email.id.clone());

        tracing::info!(
            email_id = %email.id,
            to = %email.to,
            subject = %email.subject,
            "Email sent successfully"
        );

        Ok(SendResult { message_id })
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        self.transport
            .test_connection()
            .await
            .map_err(|e| ProviderError::transient_with_source("SMTP health check failed", e))?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "smtp"
    }
}
