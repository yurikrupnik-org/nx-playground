//! Common utilities shared across all database implementations

pub mod error;

pub use core_retry::{RetryConfig, retry, retry_with_backoff};
pub use error::{DatabaseError, DatabaseResult};
