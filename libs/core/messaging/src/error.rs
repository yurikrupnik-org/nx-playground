//! Error types for message processing.

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Error categories determine retry behavior.
///
/// # Categories
///
/// - **Transient**: Temporary failure, will retry with exponential backoff
/// - **Permanent**: Unrecoverable, move to dead letter queue immediately
/// - **RateLimited**: Upstream service rate limited, retry with longer delays
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// Temporary failure (network timeout, service unavailable)
    /// Retry 3x with 1-30s exponential backoff
    Transient,

    /// Permanent failure (invalid data, missing required fields)
    /// Move to DLQ immediately, no retry
    Permanent,

    /// Rate limited by upstream service
    /// Retry 5x with 5-120s exponential backoff
    RateLimited,
}

impl ErrorCategory {
    /// Get the maximum retry count for this error category.
    pub fn max_retries(&self) -> u32 {
        match self {
            ErrorCategory::Transient => 3,
            ErrorCategory::Permanent => 0,
            ErrorCategory::RateLimited => 5,
        }
    }

    /// Get the base backoff delay in milliseconds.
    pub fn base_backoff_ms(&self) -> u64 {
        match self {
            ErrorCategory::Transient => 1000,   // 1s
            ErrorCategory::Permanent => 0,      // No retry
            ErrorCategory::RateLimited => 5000, // 5s
        }
    }

    /// Get the maximum backoff delay in milliseconds.
    pub fn max_backoff_ms(&self) -> u64 {
        match self {
            ErrorCategory::Transient => 30_000,    // 30s
            ErrorCategory::Permanent => 0,         // No retry
            ErrorCategory::RateLimited => 120_000, // 2 min
        }
    }

    /// Calculate backoff delay for a given retry count.
    pub fn backoff_delay_ms(&self, retry_count: u32) -> u64 {
        if *self == ErrorCategory::Permanent {
            return 0;
        }

        let base = self.base_backoff_ms();
        let max = self.max_backoff_ms();
        let delay = base * 2u64.saturating_pow(retry_count);
        delay.min(max)
    }

    /// Check if the job should be retried given the current retry count.
    pub fn should_retry(&self, retry_count: u32) -> bool {
        retry_count < self.max_retries()
    }

    /// Get the category as a static string (for logs and metric labels).
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCategory::Transient => "transient",
            ErrorCategory::Permanent => "permanent",
            ErrorCategory::RateLimited => "rate_limited",
        }
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error that can occur during job processing.
///
/// This error type is categorized to determine retry behavior:
/// - Transient errors will be retried with exponential backoff
/// - Permanent errors move the job to the dead letter queue
/// - Rate limited errors have longer backoff periods
#[derive(Debug, Error)]
pub enum ProcessingError {
    /// Transient error (network timeout, temporary unavailability)
    #[error("transient error: {message}")]
    Transient {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Permanent error (invalid data, business logic failure)
    #[error("permanent error: {message}")]
    Permanent {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Rate limited by upstream service
    #[error("rate limited: {message}")]
    RateLimited {
        message: String,
        retry_after_ms: Option<u64>,
    },

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Configuration error
    #[error("configuration error: {0}")]
    Config(String),

    /// Custom error with explicit category
    #[error("{message}")]
    Custom {
        category: ErrorCategory,
        message: String,
    },
}

impl ProcessingError {
    /// Create a transient error.
    pub fn transient(message: impl Into<String>) -> Self {
        Self::Transient {
            message: message.into(),
            source: None,
        }
    }

    /// Create a transient error with a source.
    pub fn transient_with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Transient {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Create a permanent error.
    pub fn permanent(message: impl Into<String>) -> Self {
        Self::Permanent {
            message: message.into(),
            source: None,
        }
    }

    /// Create a permanent error with a source.
    pub fn permanent_with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Permanent {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Create a rate limited error.
    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self::RateLimited {
            message: message.into(),
            retry_after_ms: None,
        }
    }

    /// Create a rate limited error with retry-after hint.
    pub fn rate_limited_with_retry(message: impl Into<String>, retry_after_ms: u64) -> Self {
        Self::RateLimited {
            message: message.into(),
            retry_after_ms: Some(retry_after_ms),
        }
    }

    /// Get the error category.
    pub fn category(&self) -> ErrorCategory {
        match self {
            ProcessingError::Transient { .. } => ErrorCategory::Transient,
            ProcessingError::Permanent { .. } => ErrorCategory::Permanent,
            ProcessingError::RateLimited { .. } => ErrorCategory::RateLimited,
            ProcessingError::Serialization(_) => ErrorCategory::Permanent,
            ProcessingError::Config(_) => ErrorCategory::Permanent,
            ProcessingError::Custom { category, .. } => *category,
        }
    }

    /// Check if this error should be retried.
    pub fn should_retry(&self, retry_count: u32) -> bool {
        self.category().should_retry(retry_count)
    }

    /// Calculate backoff delay for a given retry count.
    pub fn backoff_delay_ms(&self, retry_count: u32) -> u64 {
        // Use retry-after hint if available for rate limited errors
        if let ProcessingError::RateLimited {
            retry_after_ms: Some(ms),
            ..
        } = self
        {
            return *ms;
        }
        self.category().backoff_delay_ms(retry_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_category_max_retries() {
        assert_eq!(ErrorCategory::Transient.max_retries(), 3);
        assert_eq!(ErrorCategory::Permanent.max_retries(), 0);
        assert_eq!(ErrorCategory::RateLimited.max_retries(), 5);
    }

    #[test]
    fn test_error_category_backoff() {
        // Transient: 1s, 2s, 4s, 8s, 16s, max 30s
        assert_eq!(ErrorCategory::Transient.backoff_delay_ms(0), 1000);
        assert_eq!(ErrorCategory::Transient.backoff_delay_ms(1), 2000);
        assert_eq!(ErrorCategory::Transient.backoff_delay_ms(5), 30_000); // Capped

        // Permanent: no delay
        assert_eq!(ErrorCategory::Permanent.backoff_delay_ms(0), 0);

        // RateLimited: 5s, 10s, 20s, 40s, 80s, max 120s
        assert_eq!(ErrorCategory::RateLimited.backoff_delay_ms(0), 5000);
        assert_eq!(ErrorCategory::RateLimited.backoff_delay_ms(3), 40_000);
        assert_eq!(ErrorCategory::RateLimited.backoff_delay_ms(5), 120_000); // Capped
    }

    #[test]
    fn test_processing_error_category() {
        let transient = ProcessingError::transient("network timeout");
        assert_eq!(transient.category(), ErrorCategory::Transient);

        let permanent = ProcessingError::permanent("invalid email");
        assert_eq!(permanent.category(), ErrorCategory::Permanent);

        let rate_limited = ProcessingError::rate_limited("too many requests");
        assert_eq!(rate_limited.category(), ErrorCategory::RateLimited);
    }

    #[test]
    fn test_should_retry() {
        let transient = ProcessingError::transient("timeout");
        assert!(transient.should_retry(0));
        assert!(transient.should_retry(2));
        assert!(!transient.should_retry(3));

        let permanent = ProcessingError::permanent("invalid");
        assert!(!permanent.should_retry(0));
    }

    #[test]
    fn test_error_category_serialization() {
        let category = ErrorCategory::Transient;
        let json = serde_json::to_string(&category).unwrap();
        assert_eq!(json, "\"transient\"");

        let deserialized: ErrorCategory = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ErrorCategory::Transient);
    }
}
