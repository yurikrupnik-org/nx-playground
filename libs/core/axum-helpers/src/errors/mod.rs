pub mod codes;
pub mod handlers;
pub mod messages;
pub mod responses;

pub use codes::ErrorCode;

use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{DbErr, SqlxError};
use serde::Serialize;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Error as UuidError;
use validator::ValidationErrors;

/// Standard error response structure.
///
/// This structure is returned for all error responses, providing consistent
/// error information to clients including
/// - `code`: Integer error code for logging/monitoring (e.g., 1008)
/// - `error`: Machine-readable error identifier (e.g., "CONFLICT")
/// - `message`: Human-readable error message
/// - `details`: Optional additional error details (e.g., validation errors)
///
/// # JSON Example
///
/// ```json
/// {
///   "code": 1008,
///   "error": "CONFLICT",
///   "message": "Resource already exists",
///   "details": null
/// }
/// ```
#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Integer error code for logging and monitoring
    pub code: i32,
    /// Machine-readable error identifier for programmatic handling
    pub error: String,
    /// Human-readable error message
    pub message: String,
    /// Optional structured error details (e.g., validation field errors)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Application error type that can be converted to HTTP responses.
///
/// This enum integrates with common error types from dependencies
/// and provides structured error responses with error codes for observability.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AppError {
    #[error("JSON parsing error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] SqlxError),

    #[error("Migration error: {0}")]
    Migration(#[from] DbErr),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON extraction error: {0}")]
    JsonExtractorRejection(#[from] JsonRejection),

    #[error("Validation error: {0}")]
    ValidationError(#[from] ValidationErrors),

    #[error("UUID error: {0}")]
    UuidError(#[from] UuidError),

    #[error("Bad Request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Not Found: {0}")]
    NotFound(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Unprocessable Entity: {0}")]
    UnprocessableEntity(String),

    #[error("Internal Server Error: {0}")]
    InternalServerError(String),

    #[error("Service Unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("Too Many Requests: {0}")]
    TooManyRequests(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, _error_type, message, details, code) = match self {
            AppError::SerdeJson(e) => {
                tracing::error!(
                    error_code = ErrorCode::SerdeJsonError.code(),
                    "JSON parsing error: {:?}",
                    e
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalServerError",
                    ErrorCode::SerdeJsonError.default_message().to_string(),
                    None,
                    ErrorCode::SerdeJsonError,
                )
            }
            AppError::Database(e) => map_sqlx_error(&e),
            AppError::Migration(e) => {
                tracing::error!(
                    error_code = ErrorCode::MigrationError.code(),
                    "Database migration error: {:?}",
                    e
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalServerError",
                    ErrorCode::MigrationError.default_message().to_string(),
                    None,
                    ErrorCode::MigrationError,
                )
            }
            AppError::Io(e) => {
                tracing::error!(error_code = ErrorCode::IoError.code(), "I/O error: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalServerError",
                    ErrorCode::IoError.default_message().to_string(),
                    None,
                    ErrorCode::IoError,
                )
            }
            AppError::JsonExtractorRejection(e) => {
                tracing::warn!(
                    error_code = ErrorCode::JsonExtraction.code(),
                    "JSON extraction error: {:?}",
                    e
                );
                (
                    e.status(),
                    "BadRequest",
                    e.body_text(),
                    None,
                    ErrorCode::JsonExtraction,
                )
            }
            AppError::ValidationError(e) => {
                tracing::info!(
                    error_code = ErrorCode::ValidationError.code(),
                    "Validation error: {:?}",
                    e
                );
                (
                    StatusCode::BAD_REQUEST,
                    "BadRequest",
                    ErrorCode::ValidationError.default_message().to_string(),
                    Some(serde_json::to_value(&e).unwrap_or(serde_json::json!(null))),
                    ErrorCode::ValidationError,
                )
            }
            AppError::UuidError(e) => {
                tracing::warn!(
                    error_code = ErrorCode::InvalidUuid.code(),
                    "UUID error: {:?}",
                    e
                );
                (
                    StatusCode::BAD_REQUEST,
                    "BadRequest",
                    ErrorCode::InvalidUuid.default_message().to_string(),
                    None,
                    ErrorCode::InvalidUuid,
                )
            }
            AppError::BadRequest(msg) => {
                tracing::info!("Bad request: {}", msg);
                (
                    StatusCode::BAD_REQUEST,
                    "BadRequest",
                    msg,
                    None,
                    ErrorCode::InternalError,
                )
            }
            AppError::Unauthorized(msg) => {
                tracing::info!("Unauthorized: {}", msg);
                (
                    StatusCode::UNAUTHORIZED,
                    "Unauthorized",
                    msg,
                    None,
                    ErrorCode::Unauthorized,
                )
            }
            AppError::Forbidden(msg) => {
                tracing::info!("Forbidden: {}", msg);
                (
                    StatusCode::FORBIDDEN,
                    "Forbidden",
                    msg,
                    None,
                    ErrorCode::Forbidden,
                )
            }
            AppError::NotFound(msg) => {
                tracing::info!(
                    error_code = ErrorCode::NotFound.code(),
                    "Not found: {}",
                    msg
                );
                (
                    StatusCode::NOT_FOUND,
                    "NotFound",
                    msg,
                    None,
                    ErrorCode::NotFound,
                )
            }
            AppError::Conflict(msg) => {
                tracing::info!("Conflict: {}", msg);
                (
                    StatusCode::CONFLICT,
                    "Conflict",
                    msg,
                    None,
                    ErrorCode::Conflict,
                )
            }
            AppError::UnprocessableEntity(msg) => {
                tracing::info!("Unprocessable entity: {}", msg);
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "UnprocessableEntity",
                    msg,
                    None,
                    ErrorCode::UnprocessableEntity,
                )
            }
            AppError::InternalServerError(msg) => {
                tracing::error!(
                    error_code = ErrorCode::InternalError.code(),
                    "Internal server error: {}",
                    msg
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalServerError",
                    msg,
                    None,
                    ErrorCode::InternalError,
                )
            }
            AppError::ServiceUnavailable(msg) => {
                tracing::warn!("Service unavailable: {}", msg);
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "ServiceUnavailable",
                    msg,
                    None,
                    ErrorCode::ServiceUnavailable,
                )
            }
            AppError::TooManyRequests(msg) => {
                tracing::warn!("Too many requests: {}", msg);
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    "TooManyRequests",
                    msg,
                    None,
                    ErrorCode::RateLimitExceeded,
                )
            }
        };

        let body = Json(ErrorResponse {
            code: code.code(),
            error: code.as_str().to_string(),
            message,
            details,
        });

        (status, body).into_response()
    }
}

/// Maps SqlxError to appropriate HTTP response components.
///
/// This function provides detailed error handling for all SqlxError variants,
/// with appropriate status codes, messages, and error codes for observability.
fn map_sqlx_error(
    error: &SqlxError,
) -> (
    StatusCode,
    &'static str,
    String,
    Option<serde_json::Value>,
    ErrorCode,
) {
    match error {
        SqlxError::RowNotFound => {
            tracing::info!(
                error_code = ErrorCode::DatabaseNotFound.code(),
                "Database row not found"
            );
            (
                StatusCode::NOT_FOUND,
                "NotFound",
                ErrorCode::DatabaseNotFound.default_message().to_string(),
                None,
                ErrorCode::DatabaseNotFound,
            )
        }
        SqlxError::Configuration(e) => {
            tracing::error!(
                error_code = ErrorCode::DatabaseConfig.code(),
                "Database configuration error: {:?}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabaseConfig.default_message().to_string(),
                None,
                ErrorCode::DatabaseConfig,
            )
        }
        SqlxError::Database(e) => {
            tracing::error!(
                error_code = ErrorCode::DatabaseError.code(),
                "Database error: {:?}",
                e
            );
            (
                StatusCode::BAD_GATEWAY,
                "BadGateway",
                ErrorCode::DatabaseError.default_message().to_string(),
                None,
                ErrorCode::DatabaseError,
            )
        }
        SqlxError::Io(e) => {
            tracing::error!(
                error_code = ErrorCode::DatabaseIo.code(),
                "Database I/O error: {:?}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabaseIo.default_message().to_string(),
                None,
                ErrorCode::DatabaseIo,
            )
        }
        SqlxError::Tls(e) => {
            tracing::error!(
                error_code = ErrorCode::DatabaseTls.code(),
                "Database TLS error: {:?}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabaseTls.default_message().to_string(),
                None,
                ErrorCode::DatabaseTls,
            )
        }
        SqlxError::Protocol(e) => {
            tracing::error!(
                error_code = ErrorCode::DatabaseProtocol.code(),
                "Database protocol error: {:?}",
                e
            );
            (
                StatusCode::BAD_GATEWAY,
                "BadGateway",
                ErrorCode::DatabaseProtocol.default_message().to_string(),
                None,
                ErrorCode::DatabaseProtocol,
            )
        }
        SqlxError::TypeNotFound { type_name } => {
            tracing::error!(
                error_code = ErrorCode::DatabaseTypeNotFound.code(),
                "Database type not found: type_name={}",
                type_name
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabaseTypeNotFound
                    .default_message()
                    .to_string(),
                None,
                ErrorCode::DatabaseTypeNotFound,
            )
        }
        SqlxError::ColumnIndexOutOfBounds { index, len } => {
            tracing::error!(
                error_code = ErrorCode::DatabaseColumnIndex.code(),
                "Database column index out of bounds: index={}, len={}",
                index,
                len
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabaseColumnIndex.default_message().to_string(),
                None,
                ErrorCode::DatabaseColumnIndex,
            )
        }
        SqlxError::ColumnNotFound(column) => {
            tracing::error!(
                error_code = ErrorCode::DatabaseColumnNotFound.code(),
                "Database column not found: {}",
                column
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabaseColumnNotFound
                    .default_message()
                    .to_string(),
                None,
                ErrorCode::DatabaseColumnNotFound,
            )
        }
        SqlxError::Decode(e) => {
            tracing::warn!(
                error_code = ErrorCode::DatabaseDecode.code(),
                "Database decode error: {:?}",
                e
            );
            (
                StatusCode::BAD_REQUEST,
                "BadRequest",
                ErrorCode::DatabaseDecode.default_message().to_string(),
                None,
                ErrorCode::DatabaseDecode,
            )
        }
        SqlxError::Encode(e) => {
            tracing::warn!(
                error_code = ErrorCode::DatabaseEncode.code(),
                "Database encode error: {:?}",
                e
            );
            (
                StatusCode::BAD_REQUEST,
                "BadRequest",
                ErrorCode::DatabaseEncode.default_message().to_string(),
                None,
                ErrorCode::DatabaseEncode,
            )
        }
        SqlxError::AnyDriverError(e) => {
            tracing::error!(
                error_code = ErrorCode::DatabaseDriver.code(),
                "Database driver error: {:?}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabaseDriver.default_message().to_string(),
                None,
                ErrorCode::DatabaseDriver,
            )
        }
        SqlxError::PoolTimedOut => {
            tracing::warn!(
                error_code = ErrorCode::DatabasePoolTimeout.code(),
                "Database connection pool timed out"
            );
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "ServiceUnavailable",
                ErrorCode::DatabasePoolTimeout.default_message().to_string(),
                None,
                ErrorCode::DatabasePoolTimeout,
            )
        }
        SqlxError::PoolClosed => {
            tracing::error!(
                error_code = ErrorCode::DatabasePoolClosed.code(),
                "Database connection pool has been closed"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabasePoolClosed.default_message().to_string(),
                None,
                ErrorCode::DatabasePoolClosed,
            )
        }
        SqlxError::WorkerCrashed => {
            tracing::error!(
                error_code = ErrorCode::DatabaseWorkerCrashed.code(),
                "Database connection pool worker crashed"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabaseWorkerCrashed
                    .default_message()
                    .to_string(),
                None,
                ErrorCode::DatabaseWorkerCrashed,
            )
        }
        SqlxError::Migrate(e) => {
            tracing::error!(
                error_code = ErrorCode::DatabaseMigration.code(),
                "Database migration error: {:?}",
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabaseMigration.default_message().to_string(),
                None,
                ErrorCode::DatabaseMigration,
            )
        }
        _ => {
            tracing::error!(
                error_code = ErrorCode::DatabaseUnhandled.code(),
                "Unhandled database error: {:?}",
                error
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalServerError",
                ErrorCode::DatabaseUnhandled.default_message().to_string(),
                None,
                ErrorCode::DatabaseUnhandled,
            )
        }
    }
}

/// Helper function to create error responses.
///
/// # Example
///
/// ```rust,ignore
/// use axum_helpers::errors::{error_response, ErrorCode};
/// use axum::http::StatusCode;
///
/// let response = error_response(
///     StatusCode::BAD_REQUEST,
///     "Invalid input".to_string(),
///     ErrorCode::ValidationError,
/// );
/// ```
pub fn error_response(status: StatusCode, message: String, error_code: ErrorCode) -> Response {
    let body = Json(ErrorResponse {
        code: error_code.code(),
        error: error_code.as_str().to_string(),
        message,
        details: None,
    });

    (status, body).into_response()
}
