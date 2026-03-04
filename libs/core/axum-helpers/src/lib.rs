//! # Axum Helpers
//!
//! A collection of utilities, middleware, and helpers for building Axum web applications.
//!
//! ## Modules
//!
//! - **[`auth`]**: JWT authentication with Redis-backed whitelist/blacklist
//! - **[`server`]**: Server setup, health checks, graceful shutdown
//! - **[`http`]**: HTTP middleware (CORS, CSRF, security headers)
//! - **[`errors`]**: Structured error responses with error codes
//! - **[`extractors`]**: Custom extractors (UUID path, validated JSON)
//! - **[`audit`]**: Audit logging for security and compliance
//!
//! ## Quick Start
//!
//! ```ignore
//! use axum::Router;
//! use axum_helpers::server::{create_app, create_router};
//! use core_config::server::ServerConfig;
//! use utoipa::OpenApi;
//!
//! #[derive(OpenApi)]
//! #[openapi(paths())]
//! struct ApiDoc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let api_routes = Router::new(); // Add your routes
//!     let router = create_router::<ApiDoc>(api_routes).await?;
//!
//!     let config = ServerConfig::default();
//!     create_app(router, &config).await?;
//!     Ok(())
//! }
//! ```

// Domain modules
pub mod audit;
pub mod auth;
pub mod errors;
pub mod extractors;
pub mod http;
pub mod rate_limit;
pub mod server;

// Re-export auth types
pub use auth::{
    ACCESS_TOKEN_TTL, JwtClaims, JwtConfig, JwtRedisAuth, REFRESH_TOKEN_TTL, RedisAuthStore,
    jwt_auth_middleware, optional_jwt_auth_middleware,
};

// Re-export server types
pub use server::{
    CleanupCoordinator, HealthCheckFuture, HealthResponse, ReadyResponse, ShutdownCoordinator,
    create_app, create_production_app, create_router, health_router, run_health_checks,
    shutdown_signal,
};

// Re-export HTTP middleware
pub use http::{
    create_cors_layer, create_permissive_cors_layer, csrf_validation_middleware, security_headers,
};

// Re-export error types
pub use errors::{AppError, ErrorCode, ErrorResponse};

// Re-export rate limiting
pub use rate_limit::{RateLimitConfig, RateLimitTier, RateLimiter, rate_limit_middleware};

// Re-export extractors
pub use extractors::{UuidPath, ValidatedJson};

// Re-export audit types
pub use audit::{
    AuditEvent, AuditOutcome, extract_ip_from_headers, extract_ip_from_socket, extract_user_agent,
};
