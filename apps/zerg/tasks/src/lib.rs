//! Tasks gRPC service.
//!
//! ## Architecture
//!
//! ```text
//! Caller (apps/zerg/api)
//!   ↓ gRPC + `authorization: Bearer <access_token>`
//! auth.rs          - verifies the token (RS256/JWKS), derives the tenant scope
//!   ↓
//! service.rs       - proto ↔ domain conversion (contract_tasks)
//!   ↓
//! domain_tasks     - TaskService / PgTaskRepository
//!   ↓
//! PostgreSQL       - the `tasks` database, owned exclusively by this service
//! ```
//!
//! The service authenticates its own callers and derives `org_ref`/`user_ref` from the
//! verified token, so identity is not something a caller can assert. See
//! `docs/adr-tasks-service-boundary.md`.

pub mod auth;
pub mod config;
pub mod server;
pub mod service;

pub use server::run;
pub use service::TasksServiceImpl;
