//! gRPC server initialization and lifecycle management
//!
//! This module handles all server setup:
//! - Tracing initialization
//! - Database connection (PostgreSQL - the tasks-owned database)
//! - Caller-token verifier (JWKS)
//! - Service creation
//! - gRPC server configuration and startup
//! - Health check service (grpc.health.v1.Health)

use std::sync::Arc;

use contract_projects::{PROJECTS_STREAM, ProjectDeleted};
use core_config::{Environment, FromEnv};
use database::postgres::PostgresConfig;
use domain_tasks::{PgTaskRepository, TaskService};
use eyre::{Result, WrapErr};
use grpc_client::server::{GrpcServer, ServerConfig, create_health_service};
use messaging::nats::{NatsWorker, StreamConfig, WorkerConfig};
use oidc_auth::{OidcVerifier, VerifierConfig};
use rpc::tasks::v1::tasks_service_server::{SERVICE_NAME as TASKS_SERVICE, TasksServiceServer};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tonic::codec::CompressionEncoding;
use tonic::transport::Server;
use tracing::{info, warn};

use crate::auth::CallerAuth;
use crate::config::TasksAuthConfig;
use crate::project_events::{ProjectRefsProcessor, ProjectRefsStream};
use crate::service::TasksServiceImpl;

/// Run the gRPC server
///
/// This is the main entry point for server initialization. It:
/// 1. Sets up structured logging (env-aware: JSON for prod, pretty for dev)
/// 2. Connects to PostgreSQL (the tasks-owned database)
/// 3. Builds the caller-token verifier
/// 4. Creates the repository and service layers
/// 5. Starts the gRPC server with compression enabled
///
/// # Errors
///
/// Returns an error if:
/// - Database configuration is invalid
/// - Database connection fails
/// - Server binding fails
/// - Server runtime encounters an error
pub async fn run() -> Result<()> {
    // Initialize tracing (env-aware: JSON for prod, pretty for dev).
    // Guard must outlive run() so OTEL spans flush before the tokio runtime drops.
    let environment = Environment::from_env()?;
    let _tracing_guard = core_config::tracing::init_tracing(&environment, core_config::app_info!());

    // Load gRPC server configuration
    let server_config = ServerConfig::from_env().wrap_err("Failed to load server configuration")?;

    // Connect to PostgreSQL
    let db_config = PostgresConfig::from_env().wrap_err("Failed to load database configuration")?;
    info!("Connecting to PostgreSQL...");
    let db = database::postgres::connect_from_config_with_retry(db_config, None)
        .await
        .wrap_err("Failed to connect to database!")?;
    info!("Connected to PostgreSQL");

    // Caller authentication: every RPC's bearer token is verified against the IdP's
    // JWKS before the request reaches a handler, and the tenant scope is derived from
    // the verified claims rather than the request body.
    let auth_config = TasksAuthConfig::from_env().wrap_err("Failed to load auth configuration")?;
    let verifier = Arc::new(OidcVerifier::new(VerifierConfig::workos(
        &auth_config.workos_client_id,
        &auth_config.oidc_issuer,
    )));
    info!(issuer = %auth_config.oidc_issuer, "Caller token verification enabled");

    // Create tasks service
    let task_repository = PgTaskRepository::new(db);
    let task_service = TaskService::new(task_repository);
    let tasks_grpc = TasksServiceServer::new(TasksServiceImpl::new(
        task_service.clone(),
        CallerAuth::new(verifier),
    ))
    .accept_compressed(CompressionEncoding::Zstd)
    .send_compressed(CompressionEncoding::Zstd);

    // Consume `projects.>` so a deleted project's id stops dangling in our
    // tasks (backlog 0.3). Deliberately NOT a boot dependency: this service
    // must serve RPCs when NATS is down, the same reason `/ready` was ungated
    // in Phase 5. `PROJECTS` is an EventLog with a durable cursor, so a
    // consumer that starts late still applies every deletion it missed.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let refs_worker = spawn_project_refs_worker(task_service, shutdown_rx).await;

    // Create health service
    let (health_reporter, health_service) = create_health_service();
    let services = [TASKS_SERVICE];
    GrpcServer::setup_health_multiple(&health_reporter, &services).await;
    GrpcServer::log_startup_multiple(&server_config, &services);

    // Build and start server
    let addr = server_config.socket_addr();

    let served = GrpcServer::serve_with_shutdown(
        addr,
        Server::builder()
            .add_service(health_service)
            .add_service(tasks_grpc),
    )
    .await;

    // The gRPC server owns the process lifetime; once it stops, drain the
    // consumer rather than dropping it mid-message.
    let _ = shutdown_tx.send(true);
    if let Some(handle) = refs_worker {
        if let Err(e) = handle.await {
            warn!(error = %e, "project-refs worker did not shut down cleanly");
        }
    }

    served.wrap_err("gRPC server failed")?;

    Ok(())
}

/// Start the `ProjectDeleted` consumer, or return `None` when NATS is
/// unavailable.
///
/// Every failure here is logged and swallowed on purpose: a project-reference
/// correction is eventually consistent, and refusing to serve tasks because a
/// message broker is unreachable would trade a cosmetic staleness for a total
/// outage. The read side tolerates an unresolvable `project_id` regardless.
async fn spawn_project_refs_worker(
    task_service: TaskService<PgTaskRepository>,
    shutdown_rx: watch::Receiver<bool>,
) -> Option<JoinHandle<()>> {
    let nats_url = core_config::env_or_default("NATS_URL", "nats://localhost:4222");

    let jetstream = match messaging::nats::jetstream(&nats_url).await {
        Ok(js) => js,
        Err(e) => {
            warn!(
                %nats_url, error = %e,
                "NATS unavailable: project deletions will not clear task references \
                 until this service reconnects (tasks RPCs are unaffected)"
            );
            return None;
        }
    };

    let config = WorkerConfig::from_stream::<ProjectRefsStream>();
    let worker = match NatsWorker::<ProjectDeleted, _>::new(
        jetstream,
        ProjectRefsProcessor::new(task_service),
        config,
    )
    .await
    {
        Ok(w) => w,
        Err(e) => {
            warn!(error = %e, "failed to create the project-refs consumer");
            return None;
        }
    };

    info!(
        stream = PROJECTS_STREAM,
        consumer_group = ProjectRefsStream::CONSUMER_NAME,
        "consuming project events to clear deleted project references"
    );

    Some(tokio::spawn(async move {
        if let Err(e) = worker.run(shutdown_rx).await {
            warn!(error = %e, "project-refs consumer stopped with an error");
        }
    }))
}
