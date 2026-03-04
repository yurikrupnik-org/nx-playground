//! Type-safe error codes for API responses.
//!
//! This module provides a single source of truth for error codes used across
//! the application. Each error code includes:
//! - String representation for client consumption (e.g., "VALIDATION_ERROR")
//! - Integer code for logging and monitoring (e.g., 1001)
//! - Default human-readable message
//!
//! # Example
//!
//! ```rust
//! use axum_helpers::errors::ErrorCode;
//!
//! let code = ErrorCode::ValidationError;
//! assert_eq!(code.as_str(), "VALIDATION_ERROR");
//! assert_eq!(code.code(), 1001);
//! assert_eq!(code.default_message(), "Request validation failed");
//! ```

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Standardized error codes for API responses.
///
/// This enum provides a type-safe way to represent error codes across the application.
/// It combines string identifiers (for clients), integer codes (for monitoring), and
/// default messages (for consistency).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // Client errors (1000-1999)
    /// Request validation failed
    ValidationError,

    /// Invalid UUID format in path or query parameter
    InvalidUuid,

    /// Invalid JSON format in request body
    InvalidJson,

    /// Requested resource was not found
    NotFound,

    /// Authentication credentials are missing or invalid
    Unauthorized,

    /// Authenticated user lacks sufficient permissions
    Forbidden,

    /// Request conflicts with current resource state (e.g., duplicate resource)
    Conflict,

    /// Request payload is semantically incorrect
    UnprocessableEntity,

    /// JSON extraction from request body failed
    JsonExtraction,

    // Server errors (1000s)
    /// An unexpected internal server error occurred
    InternalError,

    /// Service is temporarily unavailable
    ServiceUnavailable,

    /// Rate limit exceeded
    RateLimitExceeded,

    // Database errors (2000-2999)
    /// Database query returned no results
    DatabaseNotFound,

    /// Database configuration error
    DatabaseConfig,

    /// Database connection or query error
    DatabaseError,

    /// Database I/O error
    DatabaseIo,

    /// Database TLS/SSL error
    DatabaseTls,

    /// Database protocol error
    DatabaseProtocol,

    /// Database type not found
    DatabaseTypeNotFound,

    /// Database column index out of bounds
    DatabaseColumnIndex,

    /// Database column not found
    DatabaseColumnNotFound,

    /// Failed to decode database response
    DatabaseDecode,

    /// Failed to encode database request
    DatabaseEncode,

    /// Database driver error
    DatabaseDriver,

    /// Database connection pool timed out
    DatabasePoolTimeout,

    /// Database connection pool has been closed
    DatabasePoolClosed,

    /// Database connection pool worker crashed
    DatabaseWorkerCrashed,

    /// Database migration error
    DatabaseMigration,

    /// Unhandled database error
    DatabaseUnhandled,

    // Migration errors (3000s)
    /// Database migration failed
    MigrationError,

    // I/O errors (4000s)
    /// File system I/O error
    IoError,

    // JSON parsing errors (5000s)
    /// JSON serialization/deserialization error
    SerdeJsonError,
}

impl ErrorCode {
    /// Get the string representation for client consumption.
    ///
    /// This returns a SCREAMING_SNAKE_CASE identifier that clients can use
    /// to programmatically handle specific error types.
    ///
    /// # Example
    ///
    /// ```rust
    /// use axum_helpers::errors::ErrorCode;
    ///
    /// assert_eq!(ErrorCode::ValidationError.as_str(), "VALIDATION_ERROR");
    /// assert_eq!(ErrorCode::NotFound.as_str(), "NOT_FOUND");
    /// ```
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ValidationError => "VALIDATION_ERROR",
            Self::InvalidUuid => "INVALID_UUID",
            Self::InvalidJson => "INVALID_JSON",
            Self::NotFound => "NOT_FOUND",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::Conflict => "CONFLICT",
            Self::UnprocessableEntity => "UNPROCESSABLE_ENTITY",
            Self::JsonExtraction => "JSON_EXTRACTION",
            Self::InternalError => "INTERNAL_ERROR",
            Self::ServiceUnavailable => "SERVICE_UNAVAILABLE",
            Self::RateLimitExceeded => "RATE_LIMIT_EXCEEDED",
            Self::DatabaseNotFound => "DATABASE_NOT_FOUND",
            Self::DatabaseConfig => "DATABASE_CONFIG",
            Self::DatabaseError => "DATABASE_ERROR",
            Self::DatabaseIo => "DATABASE_IO",
            Self::DatabaseTls => "DATABASE_TLS",
            Self::DatabaseProtocol => "DATABASE_PROTOCOL",
            Self::DatabaseTypeNotFound => "DATABASE_TYPE_NOT_FOUND",
            Self::DatabaseColumnIndex => "DATABASE_COLUMN_INDEX",
            Self::DatabaseColumnNotFound => "DATABASE_COLUMN_NOT_FOUND",
            Self::DatabaseDecode => "DATABASE_DECODE",
            Self::DatabaseEncode => "DATABASE_ENCODE",
            Self::DatabaseDriver => "DATABASE_DRIVER",
            Self::DatabasePoolTimeout => "DATABASE_POOL_TIMEOUT",
            Self::DatabasePoolClosed => "DATABASE_POOL_CLOSED",
            Self::DatabaseWorkerCrashed => "DATABASE_WORKER_CRASHED",
            Self::DatabaseMigration => "DATABASE_MIGRATION",
            Self::DatabaseUnhandled => "DATABASE_UNHANDLED",
            Self::MigrationError => "MIGRATION_ERROR",
            Self::IoError => "IO_ERROR",
            Self::SerdeJsonError => "SERDE_JSON_ERROR",
        }
    }

    /// Get the integer code for logging and monitoring.
    ///
    /// These codes are used in structured logs and metrics to identify error types.
    /// They are organized into ranges:
    /// - 1000-1999: Client errors
    /// - 2000-2999: Database errors
    /// - 3000-3999: Migration errors
    /// - 4000-4999: I/O errors
    /// - 5000-5999: Serialization errors
    ///
    /// # Example
    ///
    /// ```rust
    /// use axum_helpers::errors::ErrorCode;
    ///
    /// assert_eq!(ErrorCode::ValidationError.code(), 1001);
    /// assert_eq!(ErrorCode::DatabaseError.code(), 2003);
    /// ```
    pub fn code(&self) -> i32 {
        match self {
            // Client errors (1000-1999)
            Self::ValidationError => 1001,
            Self::InvalidUuid => 1002,
            Self::JsonExtraction => 1003,
            Self::NotFound => 1004,
            Self::InternalError => 1005,
            Self::Unauthorized => 1006,
            Self::Forbidden => 1007,
            Self::Conflict => 1008,
            Self::UnprocessableEntity => 1009,
            Self::InvalidJson => 1010,
            Self::ServiceUnavailable => 1011,
            Self::RateLimitExceeded => 1012,

            // Database errors (2000-2999)
            Self::DatabaseNotFound => 2001,
            Self::DatabaseConfig => 2002,
            Self::DatabaseError => 2003,
            Self::DatabaseIo => 2004,
            Self::DatabaseTls => 2005,
            Self::DatabaseProtocol => 2006,
            Self::DatabaseTypeNotFound => 2007,
            Self::DatabaseColumnIndex => 2008,
            Self::DatabaseColumnNotFound => 2009,
            Self::DatabaseDecode => 2010,
            Self::DatabaseEncode => 2011,
            Self::DatabaseDriver => 2012,
            Self::DatabasePoolTimeout => 2013,
            Self::DatabasePoolClosed => 2014,
            Self::DatabaseWorkerCrashed => 2015,
            Self::DatabaseMigration => 2016,
            Self::DatabaseUnhandled => 2099,

            // Migration errors (3000s)
            Self::MigrationError => 3001,

            // I/O errors (4000s)
            Self::IoError => 4001,

            // JSON parsing errors (5000s)
            Self::SerdeJsonError => 5001,
        }
    }

    /// Get the default user-facing error message.
    ///
    /// This provides a consistent, human-readable message for each error type.
    /// Individual handlers can override these messages with more specific details.
    ///
    /// # Example
    ///
    /// ```rust
    /// use axum_helpers::errors::ErrorCode;
    ///
    /// assert_eq!(
    ///     ErrorCode::ValidationError.default_message(),
    ///     "Request validation failed"
    /// );
    /// ```
    pub fn default_message(&self) -> &'static str {
        match self {
            Self::ValidationError => "Request validation failed",
            Self::InvalidUuid => "Invalid UUID format",
            Self::InvalidJson => "Invalid JSON format",
            Self::NotFound => "Resource not found",
            Self::Unauthorized => "Authentication required",
            Self::Forbidden => "Access forbidden",
            Self::Conflict => "Resource already exists",
            Self::UnprocessableEntity => "Request cannot be processed",
            Self::JsonExtraction => "Failed to parse request body",
            Self::InternalError => "An internal server error occurred",
            Self::ServiceUnavailable => "Service is temporarily unavailable",
            Self::RateLimitExceeded => "Rate limit exceeded",
            Self::DatabaseNotFound => "Database record not found",
            Self::DatabaseConfig => "Database configuration error",
            Self::DatabaseError => "Database error occurred",
            Self::DatabaseIo => "Database I/O error",
            Self::DatabaseTls => "Database TLS error",
            Self::DatabaseProtocol => "Database protocol error",
            Self::DatabaseTypeNotFound => "Database type not found",
            Self::DatabaseColumnIndex => "Database column index out of bounds",
            Self::DatabaseColumnNotFound => "Database column not found",
            Self::DatabaseDecode => "Failed to decode database response",
            Self::DatabaseEncode => "Failed to encode database request",
            Self::DatabaseDriver => "Database driver error",
            Self::DatabasePoolTimeout => "Database connection pool timed out",
            Self::DatabasePoolClosed => "Database connection pool closed",
            Self::DatabaseWorkerCrashed => "Database worker crashed",
            Self::DatabaseMigration => "Database migration failed",
            Self::DatabaseUnhandled => "Unhandled database error",
            Self::MigrationError => "Migration error",
            Self::IoError => "I/O error occurred",
            Self::SerdeJsonError => "JSON serialization error",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_string_representation() {
        assert_eq!(ErrorCode::ValidationError.as_str(), "VALIDATION_ERROR");
        assert_eq!(ErrorCode::NotFound.as_str(), "NOT_FOUND");
        assert_eq!(ErrorCode::DatabaseError.as_str(), "DATABASE_ERROR");
    }

    #[test]
    fn test_error_code_integer_codes() {
        assert_eq!(ErrorCode::ValidationError.code(), 1001);
        assert_eq!(ErrorCode::DatabaseError.code(), 2003);
        assert_eq!(ErrorCode::MigrationError.code(), 3001);
    }

    #[test]
    fn test_error_code_messages() {
        assert_eq!(
            ErrorCode::ValidationError.default_message(),
            "Request validation failed"
        );
        assert_eq!(ErrorCode::NotFound.default_message(), "Resource not found");
    }

    #[test]
    fn test_error_code_display() {
        assert_eq!(ErrorCode::ValidationError.to_string(), "VALIDATION_ERROR");
    }

    #[test]
    fn test_error_code_serialization() {
        let code = ErrorCode::ValidationError;
        let json = serde_json::to_string(&code).unwrap();
        assert_eq!(json, "\"VALIDATION_ERROR\"");
    }

    #[test]
    fn test_error_code_deserialization() {
        let json = "\"VALIDATION_ERROR\"";
        let code: ErrorCode = serde_json::from_str(json).unwrap();
        assert_eq!(code, ErrorCode::ValidationError);
    }
}
