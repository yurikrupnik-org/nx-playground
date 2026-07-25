use axum_helpers::{AppError, impl_into_response_via_app_error};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum UserError {
    #[error("User not found: {0}")]
    NotFound(Uuid),

    #[error("User with email '{0}' not found")]
    EmailNotFound(String),

    #[error("User with email '{0}' already exists")]
    DuplicateEmail(String),

    #[error("Invalid input: {0}")]
    Validation(String),

    #[error("Database error")]
    Database(#[from] sea_orm::DbErr),

    #[error("Redis error")]
    Redis(#[from] redis::RedisError),

    #[error("Serialization error")]
    Serialization(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type UserResult<T> = Result<T, UserError>;

impl From<UserError> for AppError {
    fn from(err: UserError) -> Self {
        match err {
            UserError::NotFound(id) => AppError::NotFound(format!("User {id} not found")),
            UserError::EmailNotFound(email) => {
                AppError::NotFound(format!("User with email '{email}' not found"))
            }
            UserError::DuplicateEmail(email) => {
                AppError::Conflict(format!("User with email '{email}' already exists"))
            }
            UserError::Validation(msg) => AppError::BadRequest(msg),
            UserError::Database(source) => {
                tracing::error!(source = %source, "Database error");
                AppError::InternalServerError("An internal error occurred".to_string())
            }
            UserError::Redis(source) => {
                tracing::error!(source = %source, "Redis error");
                AppError::InternalServerError("An internal error occurred".to_string())
            }
            UserError::Serialization(source) => {
                tracing::error!(source = %source, "Serialization error");
                AppError::InternalServerError("An internal error occurred".to_string())
            }
            UserError::Internal(msg) => {
                tracing::error!("Internal error: {msg}");
                AppError::InternalServerError("An internal error occurred".to_string())
            }
        }
    }
}

impl_into_response_via_app_error!(UserError);
