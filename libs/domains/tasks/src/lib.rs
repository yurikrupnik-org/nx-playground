//! Tasks Domain
//!
//! This module provides a complete domain implementation for managing tasks.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
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
//! use domain_tasks::{
//!     PgTaskRepository,
//!     TaskService,
//! };
//! use sea_orm::Database;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a database connection
//! let db = Database::connect("postgres://...").await?;
//!
//! // Create a repository and service
//! let repository = PgTaskRepository::new(db);
//! let service = TaskService::new(repository);
//! # Ok(())
//! # }
//! ```

pub mod conversions;
pub mod entity;
pub mod error;
pub mod handlers;
pub mod models;
#[cfg(feature = "mongo")]
pub mod mongo;
pub mod postgres;
pub mod repository;
pub mod service;

// Re-export commonly used types
pub use error::{TaskError, TaskResult};
pub use handlers::{DirectApiDoc, GrpcApiDoc};
pub use models::{
    CreateTask, Task, TaskFilter, TaskPriority, TaskResponse, TaskStatus, UpdateTask,
};
#[cfg(feature = "mongo")]
pub use mongo::MongoTaskRepository;
pub use postgres::PgTaskRepository;
pub use repository::TaskRepository;
pub use service::TaskService;

// Re-export ApiResource trait for accessing generated constants
pub use core_proc_macros::ApiResource;
