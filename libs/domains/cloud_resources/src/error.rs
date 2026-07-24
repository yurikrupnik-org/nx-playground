use axum_helpers::{AppError, impl_into_response_via_app_error};
use thiserror::Error;
use uuid::Uuid;

pub type CloudResourceResult<T> = Result<T, CloudResourceError>;

#[derive(Debug, Error)]
pub enum CloudResourceError {
    #[error("Cloud resource not found: {0}")]
    NotFound(Uuid),

    #[error("Project not found: {0}")]
    ProjectNotFound(Uuid),

    #[error("Duplicate cloud resource name: {0}")]
    DuplicateName(String),

    #[error("Invalid cloud resource status transition: {0}")]
    InvalidStatusTransition(String),

    #[error("Invalid input: {0}")]
    Validation(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),
}

impl From<CloudResourceError> for AppError {
    fn from(err: CloudResourceError) -> Self {
        match err {
            CloudResourceError::NotFound(id) => {
                AppError::NotFound(format!("Cloud resource {id} not found"))
            }
            CloudResourceError::ProjectNotFound(id) => {
                AppError::NotFound(format!("Project {id} not found"))
            }
            CloudResourceError::DuplicateName(name) => AppError::Conflict(format!(
                "Cloud resource with name '{name}' already exists in this project"
            )),
            CloudResourceError::InvalidStatusTransition(msg) => AppError::BadRequest(msg),
            CloudResourceError::Validation(msg) => AppError::BadRequest(msg),
            CloudResourceError::Internal(msg) => AppError::InternalServerError(msg),
            CloudResourceError::Database(err) => {
                AppError::InternalServerError(format!("Database error: {err}"))
            }
        }
    }
}

impl_into_response_via_app_error!(CloudResourceError);

impl From<validator::ValidationErrors> for CloudResourceError {
    fn from(err: validator::ValidationErrors) -> Self {
        CloudResourceError::Validation(err.to_string())
    }
}
