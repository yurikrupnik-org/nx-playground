//! Shared async retry with exponential backoff and jitter.
//!
//! Single source of truth for the retry logic previously duplicated in the
//! `database` and `grpc-client` crates.

use std::time::Duration;
use tracing::{debug, warn};

/// Retry configuration for transient-failure-prone operations.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,

    /// Initial delay between retries in milliseconds
    pub initial_delay_ms: u64,

    /// Maximum delay between retries in milliseconds
    pub max_delay_ms: u64,

    /// Multiplier for exponential backoff (typically 2.0)
    pub backoff_multiplier: f64,

    /// Whether to add jitter to prevent thundering herd
    pub use_jitter: bool,
}

impl RetryConfig {
    /// Create a new retry configuration with defaults.
    ///
    /// Defaults:
    /// - max_retries: 3
    /// - initial_delay_ms: 100
    /// - max_delay_ms: 5000
    /// - backoff_multiplier: 2.0
    /// - use_jitter: true
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum number of retries.
    #[must_use]
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set the initial delay between retries.
    #[must_use]
    pub fn with_initial_delay(mut self, delay_ms: u64) -> Self {
        self.initial_delay_ms = delay_ms;
        self
    }

    /// Set the maximum delay between retries.
    #[must_use]
    pub fn with_max_delay(mut self, delay_ms: u64) -> Self {
        self.max_delay_ms = delay_ms;
        self
    }

    /// Set the exponential backoff multiplier.
    #[must_use]
    pub fn with_backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier;
        self
    }

    /// Disable jitter.
    #[must_use]
    pub fn without_jitter(mut self) -> Self {
        self.use_jitter = false;
        self
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 100,
            max_delay_ms: 5000,
            backoff_multiplier: 2.0,
            use_jitter: true,
        }
    }
}

/// Retry an async operation with exponential backoff.
///
/// # Example
/// ```ignore
/// use core_retry::{retry_with_backoff, RetryConfig};
///
/// let config = RetryConfig::new().with_max_retries(5);
/// let result = retry_with_backoff(|| async { connect(&url).await }, config).await?;
/// ```
pub async fn retry_with_backoff<F, Fut, T, E>(mut operation: F, config: RetryConfig) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0;
    let mut delay = config.initial_delay_ms;

    loop {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    debug!("Operation succeeded after {} retries", attempt);
                }
                return Ok(result);
            }
            Err(e) => {
                attempt += 1;

                if attempt > config.max_retries {
                    warn!(
                        "Operation failed after {} attempts: {}",
                        config.max_retries, e
                    );
                    return Err(e);
                }

                let current_delay = if config.use_jitter {
                    apply_jitter(delay)
                } else {
                    delay
                };

                debug!(
                    "Operation failed (attempt {}/{}): {}. Retrying in {}ms...",
                    attempt, config.max_retries, e, current_delay
                );

                tokio::time::sleep(Duration::from_millis(current_delay)).await;

                delay =
                    ((delay as f64 * config.backoff_multiplier) as u64).min(config.max_delay_ms);
            }
        }
    }
}

/// Retry with the default configuration (3 attempts, 100ms initial backoff).
pub async fn retry<F, Fut, T, E>(operation: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    retry_with_backoff(operation, RetryConfig::default()).await
}

/// Apply jitter to a delay: uniform random in [50%, 100%] of the original.
fn apply_jitter(delay: u64) -> u64 {
    (delay as f64 * rand::random_range(0.5..=1.0)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn succeeds_first_try_without_retrying() {
        let calls = AtomicU32::new(0);
        let result: Result<u32, &str> = retry_with_backoff(
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(42)
            },
            RetryConfig::new().without_jitter().with_initial_delay(1),
        )
        .await;
        assert_eq!(result, Ok(42));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_until_success() {
        let calls = AtomicU32::new(0);
        let result: Result<u32, &str> = retry_with_backoff(
            || async {
                if calls.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err("transient")
                } else {
                    Ok(7)
                }
            },
            RetryConfig::new().without_jitter().with_initial_delay(1),
        )
        .await;
        assert_eq!(result, Ok(7));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn exhausts_retries_and_returns_last_error() {
        let calls = AtomicU32::new(0);
        let result: Result<u32, &str> = retry_with_backoff(
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err("down")
            },
            RetryConfig::new()
                .with_max_retries(2)
                .without_jitter()
                .with_initial_delay(1),
        )
        .await;
        assert_eq!(result, Err("down"));
        // initial attempt + 2 retries
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn builder_overrides_every_field() {
        let config = RetryConfig::new()
            .with_max_retries(5)
            .with_initial_delay(200)
            .with_max_delay(10_000)
            .with_backoff_multiplier(3.0)
            .without_jitter();

        assert_eq!(config.max_retries, 5);
        assert_eq!(config.initial_delay_ms, 200);
        assert_eq!(config.max_delay_ms, 10_000);
        assert_eq!(config.backoff_multiplier, 3.0);
        assert!(!config.use_jitter);
    }

    #[test]
    fn jitter_stays_within_bounds() {
        for _ in 0..100 {
            let jittered = apply_jitter(1000);
            assert!((500..=1000).contains(&jittered), "got {jittered}");
        }
    }
}
