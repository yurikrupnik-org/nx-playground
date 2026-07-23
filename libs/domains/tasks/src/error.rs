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
    Database(#[from] sea_orm::DbErr),
}

pub type TaskResult<T> = Result<T, TaskError>;

impl From<TaskError> for AppError {
    fn from(err: TaskError) -> Self {
        match err {
            TaskError::NotFound(id) => AppError::NotFound(format!("Task {id} not found")),
            TaskError::Validation(msg) => AppError::BadRequest(msg),
            TaskError::Internal(msg) => AppError::InternalServerError(msg),
            TaskError::Database(err) => {
                AppError::InternalServerError(format!("Database error: {err}"))
            }
        }
    }
}

impl_into_response_via_app_error!(TaskError);

/// Map a gRPC transport error onto the domain error space.
///
/// Client-input codes map to [`TaskError::Validation`] (4xx); everything else
/// is [`TaskError::Internal`]. `NotFound` needs the requested id for a useful
/// message, so use [`TaskError::from_status`] when one is in scope.
impl From<tonic::Status> for TaskError {
    fn from(status: tonic::Status) -> Self {
        match status.code() {
            tonic::Code::InvalidArgument
            | tonic::Code::FailedPrecondition
            | tonic::Code::OutOfRange => TaskError::Validation(status.message().to_owned()),
            code => TaskError::Internal(format!("gRPC {code:?}: {}", status.message())),
        }
    }
}

impl TaskError {
    /// Like the `From<tonic::Status>` impl, but maps `NotFound` to
    /// [`TaskError::NotFound`] carrying the id the caller asked for.
    pub fn from_status(status: tonic::Status, id: Uuid) -> Self {
        if status.code() == tonic::Code::NotFound {
            TaskError::NotFound(id)
        } else {
            status.into()
        }
    }
}
