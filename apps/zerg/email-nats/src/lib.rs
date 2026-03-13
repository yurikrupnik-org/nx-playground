//! Email Worker Service (NATS JetStream)
//!
//! A background worker that processes email jobs from NATS JetStream.
//!
//! ## Architecture
//!
//! ```text
//! NATS JetStream (EMAILS stream)
//!   ↓ (Pull Consumer: email-worker)
//! NatsWorker<EmailJob, EmailProcessor>
//!   ↓ (renders templates)
//! TemplateEngine (Handlebars)
//!   ↓ (sends emails)
//! EmailProvider (SendGrid/SMTP)
//!   ↓
//! Email Delivery
//! ```
//!
//! ## Features
//!
//! - NATS JetStream for durable message queues
//! - Pull-based consumer with ack/nak semantics
//! - Automatic retry with exponential backoff
//! - Dead letter queue for failed jobs
//! - Graceful shutdown handling
//! - Health check endpoints for Kubernetes probes
//! - Prometheus metrics

use core_config::{app_info, Environment};
use email::{
    EmailJob, EmailNatsStream, EmailProcessor, SendGridProvider, SmtpProvider, TemplateEngine,
};
use eyre::{Result, WrapErr};
use messaging::nats::{HealthServer, NatsWorker, WorkerConfig};
use std::time::Duration;
use tokio::signal;
use tokio::sync::watch;
use tracing::{error, info, warn};

/// Run the email worker
///
/// This is the main entry point for the NATS-based worker. It:
/// 1. Sets up structured logging (env-aware: JSON for prod, pretty for dev)
/// 2. Connects to NATS with JetStream
/// 3. Selects the appropriate email provider (SendGrid for prod, SMTP for dev)
/// 4. Starts the worker with graceful shutdown handling
///
/// # Errors
///
/// Returns an error if:
/// - NATS connection fails
/// - JetStream is not available
/// - Email provider configuration is invalid
/// - Worker encounters a fatal error
pub async fn run() -> Result<()> {
    // Initialize tracing (env-aware: JSON for prod, pretty for dev)
    let environment = Environment::from_env();
    core_config::tracing::init_tracing(&environment);

    // Initialize Prometheus metrics
    let metrics_handle = messaging::nats::metrics::init_metrics();

    // App info
    let app_info = app_info!();

    info!(
        name = %app_info.name,
        version = %app_info.version,
        "Starting NATS email worker service"
    );
    info!("Environment: {:?}", environment);

    // Health server port
    let health_port: u16 = std::env::var("EMAIL_WORKER_HEALTH_PORT")
        .or_else(|_| std::env::var("HEALTH_PORT"))
        .unwrap_or_else(|_| "8081".to_string())
        .parse()
        .unwrap_or(8081);

    // NATS connection URL
    let nats_url =
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());

    // Connect to NATS with retry (exponential backoff: 500ms, 1s, 2s, 4s, 8s, 10s cap)
    info!(url = %nats_url, "Connecting to NATS...");
    let nats_client = {
        let max_retries: u32 = 10;
        let base_delay = Duration::from_millis(500);
        let max_delay = Duration::from_secs(10);
        let mut attempt = 0u32;
        loop {
            match async_nats::connect(&nats_url).await {
                Ok(client) => break client,
                Err(e) => {
                    attempt += 1;
                    if attempt >= max_retries {
                        return Err(eyre::eyre!(
                            "Failed to connect to NATS at {} after {} attempts: {}",
                            nats_url,
                            max_retries,
                            e
                        ));
                    }
                    let delay = base_delay
                        .saturating_mul(2u32.saturating_pow(attempt - 1))
                        .min(max_delay);
                    warn!(
                        attempt,
                        max_retries,
                        delay_ms = delay.as_millis() as u64,
                        error = %e,
                        "Failed to connect to NATS, retrying..."
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    };
    info!("Connected to NATS successfully");

    // Create JetStream context
    let jetstream = async_nats::jetstream::new(nats_client);
    info!("JetStream context created");

    // Create worker configuration from EmailNatsStream
    let worker_config =
        WorkerConfig::from_stream::<EmailNatsStream>().with_health_port(health_port);

    info!(
        stream = %worker_config.stream_name,
        consumer = %worker_config.consumer_name,
        durable = %worker_config.durable_name,
        "Worker configuration loaded"
    );

    // Initialize template engine
    let templates = TemplateEngine::new().wrap_err("Failed to initialize template engine")?;
    info!("Template engine initialized");

    // Set up a shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn shutdown signal handler
    tokio::spawn(async move {
        if let Err(e) = shutdown_signal().await {
            error!("Error waiting for shutdown signal: {}", e);
        }
        let _ = shutdown_tx.send(true);
    });

    // Start health server in background
    let health_server = HealthServer::new(health_port).with_metrics(metrics_handle);
    let health_state = health_server.state();
    tokio::spawn(async move {
        if let Err(e) = health_server.run().await {
            error!(error = %e, "Health server failed");
        }
    });

    // Select email provider based on environment and run worker
    match environment {
        Environment::Production => {
            info!("Using SendGrid provider for production");
            match SendGridProvider::from_env() {
                Ok(provider) => {
                    let processor = EmailProcessor::new(provider, templates);
                    let worker =
                        NatsWorker::<EmailJob, _>::new(jetstream, processor, worker_config)
                            .await
                            .wrap_err("Failed to create NATS worker")?
                            .with_health_state(health_state);

                    info!("NATS worker created, starting processing...");
                    worker
                        .run(shutdown_rx)
                        .await
                        .map_err(|e| eyre::eyre!("{}", e))?;
                }
                Err(e) => {
                    error!("Failed to create SendGrid provider: {}", e);
                    return Err(eyre::eyre!(
                        "SendGrid configuration error: {}. Ensure SENDGRID_API_KEY and SENDGRID_FROM_EMAIL are set.",
                        e
                    ));
                }
            }
        }
        Environment::Development => {
            info!("Using SMTP provider for development (Mailpit/MailHog)");
            match SmtpProvider::mailhog() {
                Ok(provider) => {
                    let processor = EmailProcessor::new(provider, templates);
                    let worker =
                        NatsWorker::<EmailJob, _>::new(jetstream, processor, worker_config)
                            .await
                            .wrap_err("Failed to create NATS worker")?
                            .with_health_state(health_state);

                    info!("NATS worker created, starting processing...");
                    worker
                        .run(shutdown_rx)
                        .await
                        .map_err(|e| eyre::eyre!("{}", e))?;
                }
                Err(e) => {
                    error!("Failed to create SMTP provider: {}", e);
                    return Err(eyre::eyre!(
                        "SMTP configuration error: {}. Ensure SMTP_HOST and SMTP_PORT are accessible.",
                        e
                    ));
                }
            }
        }
    }

    info!("NATS email worker service stopped");
    Ok(())
}

/// Wait for a shutdown signal (SIGINT or SIGTERM)
async fn shutdown_signal() -> Result<()> {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, initiating shutdown...");
        },
        _ = terminate => {
            info!("Received SIGTERM, initiating shutdown...");
        },
    }

    Ok(())
}
