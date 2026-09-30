//! PostgreSQL database connector and utilities
//!
//! Provides connection management, migration running, and PostgreSQL-specific helpers.

#[cfg(feature = "catalog")]
pub mod catalog;
mod config;
mod connector;
mod health;

pub use config::PostgresConfig;
pub use connector::{
    connect, connect_from_config, connect_from_config_with_retry, connect_with_options,
    connect_with_retry,
};
pub use health::{HealthStatus, check_health, check_health_detailed, check_health_with_query};

// Re-export SeaORM types for convenience
pub use sea_orm::{ConnectOptions, DatabaseConnection, DbErr};
