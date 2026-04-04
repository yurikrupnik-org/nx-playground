use axum_helpers::{impl_into_response_via_app_error, AppError};
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
    Database(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Auth error: {0}")]
    Auth(String),
}

pub type TaskResult<T> = Result<T, TaskError>;

impl From<TaskError> for AppError {
    fn from(err: TaskError) -> Self {
        match err {
            TaskError::NotFound(id) => AppError::NotFound(format!("Task {} not found", id)),
            TaskError::Validation(msg) => AppError::BadRequest(msg),
            TaskError::Internal(msg) => AppError::InternalServerError(msg),
            TaskError::Database(msg) => AppError::InternalServerError(msg),
            TaskError::Network(msg) => AppError::InternalServerError(msg),
            TaskError::Auth(msg) => AppError::InternalServerError(msg),
        }
    }
}

impl_into_response_via_app_error!(TaskError);

impl From<sea_orm::DbErr> for TaskError {
    fn from(err: sea_orm::DbErr) -> Self {
        TaskError::Database(err.to_string())
    }
}
