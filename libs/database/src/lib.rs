//! Database library providing connectors and utilities for PostgreSQL and Redis
//!
//! This library provides a unified interface for connecting to and managing database
//! connections across different database types.
//!
//! # Features
//!
//! - `postgres` (default) - PostgreSQL support with SeaORM
//! - `redis` (default) - Redis support
//! - `config` - Configuration support with `core_config::FromEnv`
//! - `all` - All database features
//!
//! # Examples
//!
//! ## PostgreSQL
//!
//! ```ignore
//! use database::postgres;
//! use my_app::migrator::Migrator;
//!
//! let db = postgres::connect("postgresql://user:pass@localhost/db").await?;
//! postgres::run_migrations::<Migrator>(&db, "my_app").await?;
//! ```
//!
//! ## Redis
//!
//! ```ignore
//! use database::redis;
//! use redis::AsyncCommands;
//!
//! let mut conn = redis::connect("redis://127.0.0.1:6379").await?;
//! conn.set::<_, _, ()>("key", "value").await?;
//! ```

// Always available modules
pub mod backend;
pub mod common;

// Repository abstraction (requires postgres feature since it uses SeaORM)
#[cfg(feature = "postgres")]
pub mod repository;

// Database-specific modules (conditional based on features)
#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "mongo")]
pub mod mongo;

#[cfg(feature = "redis")]
pub mod redis;

// Introspection module (feature-gated: introspect-sqlx, introspect-sea-orm, or both)
#[cfg(any(feature = "introspect-sqlx", feature = "introspect-sea-orm"))]
pub mod introspect;

// Re-exports for convenience
pub use backend::{DatabaseBackend, DatabaseClient};
pub use common::{DatabaseError, DatabaseResult};

#[cfg(feature = "postgres")]
pub use repository::{BaseRepository, UuidEntity};
