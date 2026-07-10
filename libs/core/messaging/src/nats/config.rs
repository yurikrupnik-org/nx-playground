//! Configuration for NATS JetStream workers.

use std::time::Duration;

/// Stream configuration trait (type-safe constants).
///
/// Implement this trait to define your stream's NATS configuration.
///
/// # Example
///
/// ```rust,ignore
/// struct EmailStream;
///
/// impl StreamConfig for EmailStream {
///     const STREAM_NAME: &'static str = "EMAILS";
///     const CONSUMER_NAME: &'static str = "email-worker";
///     const DLQ_STREAM: &'static str = "EMAILS_DLQ";
///     const SUBJECT: &'static str = "emails.>";
/// }
/// ```
pub trait StreamConfig {
    /// JetStream stream name (e.g., "EMAILS")
    const STREAM_NAME: &'static str;

    /// Consumer name (e.g., "email-worker")
    const CONSUMER_NAME: &'static str;

    /// Dead letter queue stream name (e.g., "EMAILS_DLQ")
    const DLQ_STREAM: &'static str;

    /// Subject pattern (e.g., "emails.>")
    const SUBJECT: &'static str = ">";

    /// Maximum deliveries before moving to DLQ (default: 3)
    const MAX_DELIVER: i64 = 3;

    /// Ack wait timeout in seconds (default: 30)
    const ACK_WAIT_SECS: u64 = 30;

    /// Maximum pending messages (default: 1000)
    const MAX_PENDING: i64 = 1000;
}

/// Worker configuration.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// JetStream stream name
    pub stream_name: String,

    /// Consumer name
    pub consumer_name: String,

    /// Consumer durable name (unique per worker instance)
    pub durable_name: String,

    /// Subject to subscribe to
    pub subject: String,

    /// Dead letter queue stream name
    pub dlq_stream: String,

    /// Batch size for fetching messages
    pub batch_size: usize,

    /// Fetch timeout
    pub fetch_timeout: Duration,

    /// Maximum deliveries before DLQ
    pub max_deliver: i64,

    /// Ack wait timeout
    pub ack_wait: Duration,

    /// Maximum concurrent jobs (IMPROVEMENT: now actually used for parallel processing)
    pub max_concurrent_jobs: usize,

    /// Enable rate limiter
    pub enable_rate_limiter: bool,

    /// Rate limit (jobs per second)
    pub rate_limit_rps: f64,

    /// Health server port
    pub health_port: u16,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            stream_name: "JOBS".to_string(),
            consumer_name: "worker".to_string(),
            durable_name: format!("worker-{}", uuid::Uuid::new_v4()),
            subject: ">".to_string(),
            dlq_stream: "JOBS_DLQ".to_string(),
            batch_size: 10,
            fetch_timeout: Duration::from_secs(5),
            max_deliver: 3,
            ack_wait: Duration::from_secs(30),
            max_concurrent_jobs: 4,
            enable_rate_limiter: false,
            rate_limit_rps: 100.0,
            health_port: 8081,
        }
    }
}

impl WorkerConfig {
    /// Create a new worker configuration with the given stream name.
    pub fn new(stream_name: impl Into<String>) -> Self {
        let stream_name = stream_name.into();
        let dlq_stream = format!("{}_DLQ", stream_name);
        Self {
            stream_name,
            dlq_stream,
            ..Default::default()
        }
    }

    /// Create from a StreamConfig trait.
    pub fn from_stream<S: StreamConfig>() -> Self {
        Self {
            stream_name: S::STREAM_NAME.to_string(),
            consumer_name: S::CONSUMER_NAME.to_string(),
            durable_name: format!("{}-{}", S::CONSUMER_NAME, uuid::Uuid::new_v4()),
            subject: S::SUBJECT.to_string(),
            dlq_stream: S::DLQ_STREAM.to_string(),
            max_deliver: S::MAX_DELIVER,
            ack_wait: Duration::from_secs(S::ACK_WAIT_SECS),
            ..Default::default()
        }
    }

    /// Set the consumer name.
    pub fn with_consumer_name(mut self, name: impl Into<String>) -> Self {
        self.consumer_name = name.into();
        self
    }

    /// Set the durable name.
    pub fn with_durable_name(mut self, name: impl Into<String>) -> Self {
        self.durable_name = name.into();
        self
    }

    /// Set the batch size.
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Set the fetch timeout.
    pub fn with_fetch_timeout(mut self, timeout: Duration) -> Self {
        self.fetch_timeout = timeout;
        self
    }

    /// Set the maximum concurrent jobs.
    pub fn with_max_concurrent_jobs(mut self, max: usize) -> Self {
        self.max_concurrent_jobs = max;
        self
    }

    /// Enable rate limiter with the given RPS.
    pub fn with_rate_limiter(mut self, rps: f64) -> Self {
        self.enable_rate_limiter = true;
        self.rate_limit_rps = rps;
        self
    }

    /// Set the health server port.
    pub fn with_health_port(mut self, port: u16) -> Self {
        self.health_port = port;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestStream;

    impl StreamConfig for TestStream {
        const STREAM_NAME: &'static str = "TEST_JOBS";
        const CONSUMER_NAME: &'static str = "test-worker";
        const DLQ_STREAM: &'static str = "TEST_JOBS_DLQ";
        const SUBJECT: &'static str = "test.>";
        const MAX_DELIVER: i64 = 5;
    }

    #[test]
    fn test_config_from_stream() {
        let config = WorkerConfig::from_stream::<TestStream>();
        assert_eq!(config.stream_name, "TEST_JOBS");
        assert_eq!(config.consumer_name, "test-worker");
        assert_eq!(config.dlq_stream, "TEST_JOBS_DLQ");
        assert_eq!(config.subject, "test.>");
        assert_eq!(config.max_deliver, 5);
    }

    #[test]
    fn test_config_builder() {
        let config = WorkerConfig::new("MY_STREAM")
            .with_batch_size(20)
            .with_max_concurrent_jobs(8)
            .with_health_port(9090);

        assert_eq!(config.stream_name, "MY_STREAM");
        assert_eq!(config.dlq_stream, "MY_STREAM_DLQ");
        assert_eq!(config.batch_size, 20);
        assert_eq!(config.max_concurrent_jobs, 8);
        assert_eq!(config.health_port, 9090);
    }
}
