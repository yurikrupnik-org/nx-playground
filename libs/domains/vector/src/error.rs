use axum_helpers::{impl_into_response_via_app_error, AppError};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum VectorError {
    #[error("Collection not found: {0}")]
    CollectionNotFound(String),

    #[error("Vector not found: {0}")]
    VectorNotFound(Uuid),

    #[error("Invalid input: {0}")]
    Validation(String),

    #[error("Qdrant error: {0}")]
    Qdrant(#[from] qdrant_client::QdrantError),

    /// Embedding API returned an application-level failure (non-2xx response,
    /// missing embedding in the response body, ...).
    #[error("Embedding error: {0}")]
    Embedding(String),

    /// HTTP transport failure while talking to an embedding provider.
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("gRPC error: {0}")]
    Grpc(#[from] tonic::Status),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type VectorResult<T> = Result<T, VectorError>;

impl From<VectorError> for tonic::Status {
    fn from(err: VectorError) -> Self {
        match err {
            VectorError::CollectionNotFound(name) => {
                tonic::Status::not_found(format!("Collection not found: {name}"))
            }
            VectorError::VectorNotFound(id) => {
                tonic::Status::not_found(format!("Vector not found: {id}"))
            }
            VectorError::Validation(msg) => tonic::Status::invalid_argument(msg),
            VectorError::Qdrant(err) => tonic::Status::internal(format!("Qdrant error: {err}")),
            VectorError::Embedding(msg) => {
                tonic::Status::internal(format!("Embedding error: {msg}"))
            }
            VectorError::Http(err) => {
                tonic::Status::internal(format!("HTTP request failed: {err}"))
            }
            VectorError::Json(err) => tonic::Status::internal(format!("JSON error: {err}")),
            VectorError::Grpc(status) => status,
            VectorError::Config(msg) => {
                tonic::Status::failed_precondition(format!("Config error: {msg}"))
            }
            VectorError::Internal(msg) => tonic::Status::internal(msg),
        }
    }
}

/// Convert VectorError to AppError for standardized HTTP error responses
impl From<VectorError> for AppError {
    fn from(err: VectorError) -> Self {
        match err {
            VectorError::CollectionNotFound(name) => {
                AppError::NotFound(format!("Collection {name} not found"))
            }
            VectorError::VectorNotFound(id) => AppError::NotFound(format!("Vector {id} not found")),
            VectorError::Validation(msg) => AppError::BadRequest(msg),
            VectorError::Qdrant(e) => AppError::InternalServerError(format!("Qdrant error: {e}")),
            VectorError::Embedding(msg) => {
                AppError::InternalServerError(format!("Embedding error: {msg}"))
            }
            VectorError::Http(e) => {
                AppError::InternalServerError(format!("HTTP request failed: {e}"))
            }
            VectorError::Json(e) => AppError::InternalServerError(format!("JSON error: {e}")),
            VectorError::Grpc(status) => {
                AppError::InternalServerError(format!("gRPC error: {status}"))
            }
            VectorError::Config(msg) => {
                AppError::InternalServerError(format!("Config error: {msg}"))
            }
            VectorError::Internal(msg) => AppError::InternalServerError(msg),
        }
    }
}

impl_into_response_via_app_error!(VectorError);
