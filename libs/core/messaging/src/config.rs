//! Configuration types for message queues.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Queue configuration.
///
/// This is a backend-agnostic configuration that can be used to configure
/// NATS JetStream workers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    /// Queue/stream name
    pub queue_name: String,

    /// Consumer group name
    pub consumer_group: String,

    /// Consumer ID (unique per worker instance)
    pub consumer_id: String,

    /// Dead letter queue name
    pub dlq_name: String,

    /// Maximum queue length before trimming
    pub max_length: i64,

    /// Poll interval when no messages are available (milliseconds)
    #[serde(with = "duration_millis")]
    pub poll_interval: Duration,

    /// Batch size for reading messages
    pub batch_size: usize,

    /// Blocking timeout for read operations (None = non-blocking)
    #[serde(with = "option_duration_millis")]
    pub blocking_timeout: Option<Duration>,

    /// Maximum concurrent jobs per worker
    pub max_concurrent_jobs: usize,

    /// Timeout for claiming abandoned messages
    #[serde(with = "duration_millis")]
    pub claim_timeout: Duration,

    /// Retry policy
    pub retry_policy: RetryPolicy,

    /// Enable circuit breaker (TODO: implement)
    pub enable_circuit_breaker: bool,

    /// Enable rate limiter
    pub enable_rate_limiter: bool,

    /// Rate limit (requests per second)
    pub rate_limit_rps: f64,
}

// Custom serde for Duration as milliseconds
mod duration_millis {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

mod option_duration_millis {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.map(|d| d.as_millis() as u64).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis: Option<u64> = Option::deserialize(deserializer)?;
        Ok(millis.map(Duration::from_millis))
    }
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            queue_name: "default:queue".to_string(),
            consumer_group: "default_workers".to_string(),
            consumer_id: format!("worker-{}", uuid::Uuid::new_v4()),
            dlq_name: "default:dlq".to_string(),
            max_length: 100_000,
            poll_interval: Duration::from_secs(1),
            batch_size: 10,
            blocking_timeout: Some(Duration::from_secs(5)),
            max_concurrent_jobs: 4,
            claim_timeout: Duration::from_secs(30),
            retry_policy: RetryPolicy::default(),
            enable_circuit_breaker: true,
            enable_rate_limiter: false,
            rate_limit_rps: 100.0,
        }
    }
}

impl QueueConfig {
    /// Create a new queue configuration with the given queue name.
    pub fn new(queue_name: impl Into<String>) -> Self {
        let queue_name = queue_name.into();
        let dlq_name = format!("{}:dlq", queue_name);
        Self {
            queue_name,
            dlq_name,
            ..Default::default()
        }
    }

    /// Create from queue definition trait constants.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = QueueConfig::from_def::<EmailStream>();
    /// ```
    pub fn from_def<D: QueueDef>() -> Self {
        Self {
            queue_name: D::QUEUE_NAME.to_string(),
            consumer_group: D::CONSUMER_GROUP.to_string(),
            consumer_id: format!("{}-{}", D::CONSUMER_GROUP, uuid::Uuid::new_v4()),
            dlq_name: D::DLQ_NAME.to_string(),
            max_length: D::MAX_LENGTH,
            poll_interval: Duration::from_millis(D::POLL_INTERVAL_MS),
            batch_size: D::BATCH_SIZE,
            claim_timeout: Duration::from_millis(D::CLAIM_TIMEOUT_MS),
            ..Default::default()
        }
    }

    /// Set the consumer group.
    pub fn with_consumer_group(mut self, group: impl Into<String>) -> Self {
        self.consumer_group = group.into();
        self
    }

    /// Set the consumer ID.
    pub fn with_consumer_id(mut self, id: impl Into<String>) -> Self {
        self.consumer_id = id.into();
        self
    }

    /// Set the blocking timeout.
    pub fn with_blocking_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.blocking_timeout = timeout;
        self
    }

    /// Set the batch size.
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Set the maximum concurrent jobs.
    pub fn with_max_concurrent_jobs(mut self, max: usize) -> Self {
        self.max_concurrent_jobs = max;
        self
    }

    /// Enable circuit breaker.
    pub fn with_circuit_breaker(mut self, enabled: bool) -> Self {
        self.enable_circuit_breaker = enabled;
        self
    }

    /// Enable rate limiter with the given RPS.
    pub fn with_rate_limiter(mut self, rps: f64) -> Self {
        self.enable_rate_limiter = true;
        self.rate_limit_rps = rps;
        self
    }

    /// Set the retry policy.
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }
}

/// Queue definition trait (for type-safe constants).
///
/// Implement this trait to define your queue's configuration constants.
pub trait QueueDef {
    /// Queue/stream name
    const QUEUE_NAME: &'static str;

    /// Consumer group name
    const CONSUMER_GROUP: &'static str;

    /// Dead letter queue name
    const DLQ_NAME: &'static str;

    /// Maximum queue length before trimming (default: 100,000)
    const MAX_LENGTH: i64 = 100_000;

    /// Poll interval in milliseconds (default: 1000)
    const POLL_INTERVAL_MS: u64 = 1000;

    /// Batch size for reading messages (default: 10)
    const BATCH_SIZE: usize = 10;

    /// Claim timeout in milliseconds (default: 30,000)
    const CLAIM_TIMEOUT_MS: u64 = 30_000;
}

/// Retry policy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum retries for transient errors
    pub max_transient_retries: u32,

    /// Maximum retries for rate limited errors
    pub max_rate_limit_retries: u32,

    /// Backoff strategy
    pub backoff: BackoffStrategy,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_transient_retries: 3,
            max_rate_limit_retries: 5,
            backoff: BackoffStrategy::Exponential {
                base_ms: 1000,
                max_ms: 30000,
            },
        }
    }
}

/// Backoff strategy for retries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackoffStrategy {
    /// Fixed delay between retries
    Fixed { delay_ms: u64 },

    /// Exponential backoff (base * 2^retry_count, capped at max)
    Exponential { base_ms: u64, max_ms: u64 },

    /// Linear backoff (base * (retry_count + 1), capped at max)
    Linear { base_ms: u64, max_ms: u64 },
}

impl BackoffStrategy {
    /// Calculate the delay for a given retry count.
    pub fn delay(&self, retry_count: u32) -> Duration {
        match self {
            BackoffStrategy::Fixed { delay_ms } => Duration::from_millis(*delay_ms),
            BackoffStrategy::Exponential { base_ms, max_ms } => {
                let delay = base_ms.saturating_mul(2u64.saturating_pow(retry_count));
                Duration::from_millis(delay.min(*max_ms))
            }
            BackoffStrategy::Linear { base_ms, max_ms } => {
                let delay = base_ms.saturating_mul((retry_count + 1) as u64);
                Duration::from_millis(delay.min(*max_ms))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestQueue;

    impl QueueDef for TestQueue {
        const QUEUE_NAME: &'static str = "test:queue";
        const CONSUMER_GROUP: &'static str = "test_workers";
        const DLQ_NAME: &'static str = "test:dlq";
        const MAX_LENGTH: i64 = 50_000;
    }

    #[test]
    fn test_config_from_def() {
        let config = QueueConfig::from_def::<TestQueue>();
        assert_eq!(config.queue_name, "test:queue");
        assert_eq!(config.consumer_group, "test_workers");
        assert_eq!(config.dlq_name, "test:dlq");
        assert_eq!(config.max_length, 50_000);
    }

    #[test]
    fn test_backoff_exponential() {
        let backoff = BackoffStrategy::Exponential {
            base_ms: 1000,
            max_ms: 30000,
        };

        assert_eq!(backoff.delay(0), Duration::from_secs(1));
        assert_eq!(backoff.delay(1), Duration::from_secs(2));
        assert_eq!(backoff.delay(2), Duration::from_secs(4));
        assert_eq!(backoff.delay(5), Duration::from_secs(30)); // Capped
    }

    #[test]
    fn test_backoff_linear() {
        let backoff = BackoffStrategy::Linear {
            base_ms: 5000,
            max_ms: 60000,
        };

        assert_eq!(backoff.delay(0), Duration::from_secs(5)); // 5 * 1
        assert_eq!(backoff.delay(1), Duration::from_secs(10)); // 5 * 2
        assert_eq!(backoff.delay(2), Duration::from_secs(15)); // 5 * 3
        assert_eq!(backoff.delay(20), Duration::from_secs(60)); // Capped
    }

    #[test]
    fn test_config_serialization() {
        let config = QueueConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: QueueConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.queue_name, deserialized.queue_name);
    }
}
