//! Application state management.
//!
//! This module defines the shared application state passed to all request handlers.
//! The state contains:
//! - Configuration
//! - gRPC client connections
//! - Database connections (PostgreSQL, Redis)
//! - Notification service (NATS-based email queueing)
//! - Vector service (Qdrant-backed)

use axum_helpers::RateLimiter;
use domain_vector::{QdrantRepository, VectorService};
use email::NotificationService;
use grpc_client::TracedChannel;
use rpc::tasks::tasks_service_client::TasksServiceClient;
use std::sync::Arc;
use tonic::transport::Channel;
use tonic_health::pb::health_client::HealthClient;

/// Shared application state.
///
/// This struct is cloned for each handler (inexpensive Arc clones), providing access to:
/// - Application configuration
/// - gRPC tasks service client (cheap to clone, shares underlying connection)
/// - PostgreSQL database connection pool (SeaORM)
/// - Redis connection manager
/// - WorkOS OIDC auth (verifier + server-side sessions + login-flow store)
/// - Notification service for email queueing via NATS
/// - Vector service for Qdrant operations
#[derive(Clone)]
pub struct AppState {
    /// Application configuration loaded from environment variables
    pub config: crate::config::Config,
    /// gRPC client for the task service (cloneable, shares HTTP/2 connection pool)
    /// No lock needed - cloning is cheap and thread-safe
    pub tasks_client: TasksServiceClient<TracedChannel>,
    /// gRPC health client for the task service, sharing the same lazy channel.
    /// Used by `/ready` to gate traffic on tasks reachability without blocking startup.
    pub tasks_health: HealthClient<Channel>,
    /// PostgreSQL database connection pool (SeaORM)
    pub db: database::postgres::DatabaseConnection,
    /// Redis connection manager
    pub redis: database::redis::ConnectionManager,
    /// Single-use PKCE/CSRF login-flow store (authorize → callback window)
    pub flows: oidc_auth::LoginFlowStore,
    /// Opaque server-side session store (token-handler / BFF model)
    pub sessions: Arc<oidc_auth::RedisSessionStore>,
    /// WorkOS AuthKit login/acquisition provider
    pub provider: Arc<oidc_auth::WorkosProvider>,
    /// JWKS/RS256 bearer-token verifier
    pub verifier: Arc<oidc_auth::OidcVerifier>,
    /// Notification service for queueing emails via NATS JetStream
    pub notifications: NotificationService,
    /// Vector service for Qdrant operations (wrapped in Arc for cheap cloning)
    pub vector_service: Option<Arc<VectorService<QdrantRepository>>>,
    /// Distributed rate limiter (Redis-backed sliding window counter)
    pub rate_limiter: RateLimiter,
}
