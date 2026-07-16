use axum_helpers::{impl_into_response_via_app_error, AppError};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum TodoError {
    #[error("Todo not found: {0}")]
    NotFound(Uuid),

    #[error("Invalid input: {0}")]
    Validation(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    Database(String),
}

pub type TodoResult<T> = Result<T, TodoError>;

impl From<TodoError> for AppError {
    fn from(err: TodoError) -> Self {
        match err {
            TodoError::NotFound(id) => AppError::NotFound(format!("Todo {id} not found")),
            TodoError::Validation(msg) => AppError::BadRequest(msg),
            TodoError::Internal(msg) => AppError::InternalServerError(msg),
            TodoError::Database(msg) => {
                AppError::InternalServerError(format!("Database error: {msg}"))
            }
        }
    }
}

impl_into_response_via_app_error!(TodoError);

impl From<sea_orm::DbErr> for TodoError {
    fn from(err: sea_orm::DbErr) -> Self {
        TodoError::Database(err.to_string())
    }
}

impl From<validator::ValidationErrors> for TodoError {
    fn from(err: validator::ValidationErrors) -> Self {
        TodoError::Validation(err.to_string())
    }
}
