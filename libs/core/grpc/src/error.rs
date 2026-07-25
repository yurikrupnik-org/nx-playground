use std::time::Duration;
use thiserror::Error;

pub type GrpcResult<T> = Result<T, GrpcError>;

/// Errors that can occur during gRPC client creation and configuration
#[derive(Error, Debug)]
pub enum GrpcError {
    /// Invalid URI provided for connection
    #[error("invalid URI")]
    InvalidUri(#[source] tonic::transport::Error),

    /// Failed to establish connection
    #[error("connection failed")]
    ConnectionFailed(#[source] tonic::transport::Error),

    /// Connection timeout
    #[error("Connection timeout after {0:?}")]
    ConnectionTimeout(Duration),

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Maximum retries exceeded
    #[error("Maximum retries ({0}) exceeded")]
    MaxRetriesExceeded(u32),
}

// Implement conversion to tonic::Status for use in interceptors
impl From<GrpcError> for tonic::Status {
    fn from(err: GrpcError) -> Self {
        // Status messages are plain strings, so render the whole source chain
        // (thiserror keeps sources out of Display to avoid duplication).
        let mut message = err.to_string();
        let mut source = std::error::Error::source(&err);
        while let Some(cause) = source {
            message.push_str(": ");
            message.push_str(&cause.to_string());
            source = cause.source();
        }

        match err {
            GrpcError::InvalidUri(_) | GrpcError::InvalidConfig(_) => {
                tonic::Status::invalid_argument(message)
            }
            GrpcError::ConnectionFailed(_)
            | GrpcError::ConnectionTimeout(_)
            | GrpcError::MaxRetriesExceeded(_) => tonic::Status::unavailable(message),
        }
    }
}

// ============================================================================
// Generic Error Conversion Traits
// ============================================================================

/// Extension trait for Result types to convert errors to tonic::Status
///
/// This trait provides ergonomic methods to convert domain errors into gRPC Status errors.
///
/// # Example
/// ```ignore
/// use grpc_client::error::ToTonicResult;
///
/// fn validate_input(input: &str) -> Result<String, String> {
///     if input.is_empty() {
///         Err("Input cannot be empty".to_string())
///     } else {
///         Ok(input.to_string())
///     }
/// }
///
/// // Convert String error to tonic::Status
/// let result = validate_input("").to_tonic();
/// ```
pub trait ToTonicResult<T> {
    /// Convert the error in this Result to a tonic::Status with INVALID_ARGUMENT code
    fn to_tonic(self) -> Result<T, tonic::Status>;

    /// Convert the error to a tonic::Status with a custom code
    fn to_tonic_with_code(self, code: tonic::Code) -> Result<T, tonic::Status>;
}

impl<T> ToTonicResult<T> for Result<T, String> {
    fn to_tonic(self) -> Result<T, tonic::Status> {
        self.map_err(tonic::Status::invalid_argument)
    }

    fn to_tonic_with_code(self, code: tonic::Code) -> Result<T, tonic::Status> {
        self.map_err(|e| tonic::Status::new(code, e))
    }
}

/// Extension trait for Option types to convert None to tonic::Status errors
///
/// # Example
/// ```ignore
/// use grpc_client::error::ToTonicOption;
///
/// let user_id: Option<String> = None;
/// let result = user_id.ok_or_not_found("User not found");
/// ```
pub trait ToTonicOption<T> {
    /// Convert None to a tonic::Status with NOT_FOUND code
    fn ok_or_not_found(self, message: impl Into<String>) -> Result<T, tonic::Status>;

    /// Convert None to a tonic::Status with INVALID_ARGUMENT code
    fn ok_or_invalid(self, message: impl Into<String>) -> Result<T, tonic::Status>;
}

impl<T> ToTonicOption<T> for Option<T> {
    fn ok_or_not_found(self, message: impl Into<String>) -> Result<T, tonic::Status> {
        self.ok_or_else(|| tonic::Status::not_found(message.into()))
    }

    fn ok_or_invalid(self, message: impl Into<String>) -> Result<T, tonic::Status> {
        self.ok_or_else(|| tonic::Status::invalid_argument(message.into()))
    }
}
