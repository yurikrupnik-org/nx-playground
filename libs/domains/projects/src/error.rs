use axum_helpers::{AppError, impl_into_response_via_app_error};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("Project not found: {0}")]
    NotFound(Uuid),

    #[error("Project with name '{0}' already exists for this user")]
    DuplicateName(String),

    #[error("Invalid input: {0}")]
    Validation(String),

    #[error("Unauthorized access to project {0}")]
    Unauthorized(Uuid),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),
}

pub type ProjectResult<T> = Result<T, ProjectError>;

impl From<ProjectError> for AppError {
    fn from(err: ProjectError) -> Self {
        match err {
            ProjectError::NotFound(id) => AppError::NotFound(format!("Project {id} not found")),
            ProjectError::DuplicateName(name) => {
                AppError::Conflict(format!("Project with name '{name}' already exists"))
            }
            ProjectError::Validation(msg) => AppError::BadRequest(msg),
            ProjectError::Unauthorized(id) => {
                AppError::Forbidden(format!("Access denied to project {id}"))
            }
            ProjectError::Internal(msg) => AppError::InternalServerError(msg),
            ProjectError::Database(err) => {
                AppError::InternalServerError(format!("Database error: {err}"))
            }
        }
    }
}

impl_into_response_via_app_error!(ProjectError);
