//! Projects Domain
//!
//! This module provides a complete domain implementation for managing cloud projects.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
//! │  Handlers   │  ← HTTP/gRPC endpoints
//! └──────┬──────┘
//!        │
//! ┌──────▼──────┐
//! │   Service   │  ← Business logic, validation
//! └──────┬──────┘
//!        │
//! ┌──────▼──────┐
//! │ Repository  │  ← Data access (trait + implementations)
//! └──────┬──────┘
//!        │
//! ┌──────▼──────┐
//! │   Models    │  ← Entities, DTOs, enums
//! └─────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! use domain_projects::{
//!     NoopProjectPublisher,
//!     handlers,
//!     postgres::PgProjectRepository,
//!     service::ProjectService,
//! };
//! use sea_orm::Database;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a database connection
//! let db = Database::connect("postgres://...").await?;
//!
//! // Create a repository and service. The publisher is explicit: production
//! // passes `NatsProjectPublisher` so `ProjectDeleted` reaches the tasks
//! // service, and there is no default that silently drops it.
//! let repository = PgProjectRepository::new(db);
//! let service = ProjectService::new(repository, Arc::new(NoopProjectPublisher));
//!
//! // Create Axum router
//! let router = handlers::router(service);
//! # Ok(())
//! # }
//! ```

pub mod entity;
pub mod error;
pub mod events;
pub mod handlers;
pub mod models;
pub mod nats;
pub mod postgres;
pub mod repository;
pub mod service;

// Re-export commonly used types
pub use error::{ProjectError, ProjectResult};
pub use events::{NoopProjectPublisher, ProjectEventPublisher};
pub use handlers::ApiDoc;
pub use models::{
    CloudProvider, CreateProject, Environment, Project, ProjectFilter, ProjectStatus, Tag,
    UpdateProject,
};
pub use nats::NatsProjectPublisher;
pub use postgres::PgProjectRepository;
pub use repository::ProjectRepository;
pub use service::ProjectService;

// Re-export ApiResource trait for accessing generated constants
pub use core_proc_macros::ApiResource;
