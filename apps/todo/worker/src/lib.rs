//! Todo event worker.
//!
//! Consumes the `TODOS` JetStream stream (`todos.>`) and runs [`TodoProcessor`]
//! against each [`TodoEvent`]. Mirrors the email worker: pull consumer with
//! ack/nak/DLQ, Prometheus metrics, k8s health probes, graceful shutdown.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use config::AppConfig;
use core_config::{app_info, FromEnv};
use domain_todo::{TodoEvent, TodoNatsStream};
use eyre::{Result, WrapErr};
use messaging::nats::{HealthServer, NatsWorker, WorkerConfig};
use messaging::{ProcessingError, Processor};
use tokio::signal;
use tokio::sync::watch;
use tracing::{error, info};

mod config;

/// Processes todo lifecycle events. Keeps a running count of events handled and
/// of completions, exposed for assertions/observability.
#[derive(Clone, Default)]
pub struct TodoProcessor {
    processed: Arc<AtomicU64>,
    completed: Arc<AtomicU64>,
}

impl TodoProcessor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Total events processed so far.
    pub fn processed_count(&self) -> u64 {
        self.processed.load(Ordering::SeqCst)
    }

    /// Number of `Completed` events seen so far (a simple read-model projection).
    pub fn completed_count(&self) -> u64 {
        self.completed.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Processor<TodoEvent> for TodoProcessor {
    #[tracing::instrument(skip_all, fields(todo_id = %job.todo_id, event_id = %job.event_id))]
    async fn process(&self, job: &TodoEvent) -> Result<(), ProcessingError> {
        info!(
            kind = ?job.kind,
            todo_id = %job.todo_id,
            event_id = %job.event_id,
            "processing todo event"
        );
        self.processed.fetch_add(1, Ordering::SeqCst);
        if matches!(job.kind, domain_todo::TodoEventKind::Completed) {
            self.completed.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "todo_processor"
    }
}

/// Run the todo worker until a shutdown signal is received.
pub async fn run() -> Result<()> {
    let config = AppConfig::from_env()?;
    let _tracing_guard = core_config::tracing::init_tracing(&config.environment, app_info!());

    let metrics_handle = messaging::nats::metrics::init_metrics();

    let health_port = config.health_port;
    let nats_url = config.nats_url;

    info!(%nats_url, "connecting to NATS...");
    let jetstream = messaging::nats::jetstream_with_retry(&nats_url, None)
        .await
        .wrap_err("failed to connect to NATS")?;
    info!("JetStream context created");

    // Durable name comes from TodoNatsStream::CONSUMER_NAME ("todo-worker"); the lib
    // uses a stable durable so restarts/replicas share one consumer (a work queue).
    let worker_config = WorkerConfig::from_stream::<TodoNatsStream>().with_health_port(health_port);
    info!(
        stream = %worker_config.stream_name,
        durable = %worker_config.durable_name,
        "todo worker configuration loaded"
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        if let Err(e) = shutdown_signal().await {
            error!(error = %e, "error waiting for shutdown signal");
        }
        let _ = shutdown_tx.send(true);
    });

    let health_server = HealthServer::new(health_port).with_metrics(metrics_handle);
    let health_state = health_server.state();
    tokio::spawn(async move {
        if let Err(e) = health_server.run().await {
            error!(error = %e, "health server failed");
        }
    });

    let processor = TodoProcessor::new();
    let worker = NatsWorker::<TodoEvent, _>::new(jetstream, processor, worker_config)
        .await
        .wrap_err("failed to create NATS worker")?
        .with_health_state(health_state);

    info!("todo worker created, starting processing...");
    worker
        .run(shutdown_rx)
        .await
        .map_err(|e| eyre::eyre!("{e}"))?;

    info!("todo worker stopped");
    Ok(())
}

/// Wait for SIGINT or SIGTERM.
pub async fn shutdown_signal() -> Result<()> {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    Ok(())
}
