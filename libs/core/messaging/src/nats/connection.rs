//! NATS connection helpers with optional retry/backoff.
//!
//! Mirrors the ergonomics of the `database` crate's `connect` / `connect_with_retry`
//! so every service establishes NATS connections the same way instead of hand-rolling
//! `async_nats::connect` + backoff loops at each call site.

use crate::nats::error::NatsError;
use async_nats::Client;
use async_nats::jetstream::Context;
use std::time::Duration;
use tracing::{info, warn};

/// Retry policy for NATS connection attempts (bounded exponential backoff).
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of connection attempts before giving up.
    pub max_retries: u32,
    /// Delay before the first retry, in milliseconds.
    pub initial_delay_ms: u64,
    /// Upper bound on the delay between retries, in milliseconds.
    pub max_delay_ms: u64,
    /// Exponential backoff multiplier (typically 2.0).
    pub backoff_multiplier: f64,
}

impl RetryConfig {
    /// Retry policy with defaults (10 attempts, 500ms initial delay, 10s cap).
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the maximum number of connection attempts.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Override the initial retry delay (milliseconds).
    pub fn with_initial_delay(mut self, delay_ms: u64) -> Self {
        self.initial_delay_ms = delay_ms;
        self
    }

    /// Override the maximum retry delay (milliseconds).
    pub fn with_max_delay(mut self, delay_ms: u64) -> Self {
        self.max_delay_ms = delay_ms;
        self
    }

    /// Delay before the given (1-based) retry attempt, capped at `max_delay_ms`.
    fn delay_for(&self, attempt: u32) -> Duration {
        let factor = self
            .backoff_multiplier
            .powi(attempt.saturating_sub(1) as i32);
        let ms = (self.initial_delay_ms as f64 * factor).min(self.max_delay_ms as f64);
        Duration::from_millis(ms as u64)
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            initial_delay_ms: 500,
            max_delay_ms: 10_000,
            backoff_multiplier: 2.0,
        }
    }
}

/// Connect to NATS with a single attempt.
pub async fn connect(url: &str) -> Result<Client, NatsError> {
    let client = async_nats::connect(url).await?;
    info!(nats_url = %url, "connected to NATS");
    Ok(client)
}

/// Connect to NATS with bounded exponential backoff.
///
/// Pass `None` for the default policy (10 attempts, 500ms -> 10s cap).
pub async fn connect_with_retry(
    url: &str,
    retry: Option<RetryConfig>,
) -> Result<Client, NatsError> {
    let cfg = retry.unwrap_or_default();
    let mut attempt = 0u32;
    loop {
        match async_nats::connect(url).await {
            Ok(client) => {
                info!(nats_url = %url, attempt, "connected to NATS");
                return Ok(client);
            }
            Err(e) => {
                attempt += 1;
                if attempt >= cfg.max_retries {
                    return Err(NatsError::Connection(e));
                }
                let delay = cfg.delay_for(attempt);
                warn!(
                    attempt,
                    max_retries = cfg.max_retries,
                    delay_ms = delay.as_millis() as u64,
                    error = %e,
                    "NATS connect failed, retrying..."
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Connect to NATS (single attempt) and open a JetStream context.
pub async fn jetstream(url: &str) -> Result<Context, NatsError> {
    Ok(async_nats::jetstream::new(connect(url).await?))
}

/// Connect to NATS with retry and open a JetStream context.
pub async fn jetstream_with_retry(
    url: &str,
    retry: Option<RetryConfig>,
) -> Result<Context, NatsError> {
    Ok(async_nats::jetstream::new(
        connect_with_retry(url, retry).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_matches_documented_values() {
        let cfg = RetryConfig::default();
        assert_eq!(cfg.max_retries, 10);
        assert_eq!(cfg.initial_delay_ms, 500);
        assert_eq!(cfg.max_delay_ms, 10_000);
        assert_eq!(cfg.backoff_multiplier, 2.0);
    }

    #[test]
    fn builders_override_fields() {
        let cfg = RetryConfig::new()
            .with_max_retries(3)
            .with_initial_delay(100)
            .with_max_delay(2_000);
        assert_eq!(cfg.max_retries, 3);
        assert_eq!(cfg.initial_delay_ms, 100);
        assert_eq!(cfg.max_delay_ms, 2_000);
    }

    #[test]
    fn delay_grows_exponentially_then_caps() {
        let cfg = RetryConfig::default();
        // 500ms, 1s, 2s, 4s, 8s, then capped at 10s.
        assert_eq!(cfg.delay_for(1).as_millis(), 500);
        assert_eq!(cfg.delay_for(2).as_millis(), 1_000);
        assert_eq!(cfg.delay_for(3).as_millis(), 2_000);
        assert_eq!(cfg.delay_for(4).as_millis(), 4_000);
        assert_eq!(cfg.delay_for(5).as_millis(), 8_000);
        assert_eq!(cfg.delay_for(6).as_millis(), 10_000);
        assert_eq!(cfg.delay_for(20).as_millis(), 10_000);
    }
}
