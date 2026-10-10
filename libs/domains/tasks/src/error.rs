//! Server-side domain errors for tasks.
//!
//! This crate is the tasks *service*: it owns storage and business rules and knows
//! nothing about transports. Mapping these onto a wire status is the hosting binary's
//! job (`apps/zerg/tasks`), and mapping a wire status onto HTTP is the caller's
//! (`apps/zerg/api`). See `docs/adr-tasks-service-boundary.md`.

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("Task not found: {0}")]
    NotFound(Uuid),

    #[error("Invalid input: {0}")]
    Validation(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),
}

pub type TaskResult<T> = Result<T, TaskError>;
