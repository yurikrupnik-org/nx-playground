//! Configuration for NATS JetStream workers.

use std::time::Duration;

/// How a stream's messages may be consumed.
///
/// This is the *stream's* nature, independent of how many replicas a worker runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    /// Exclusive job queue: exactly one consumer group, and a message is deleted
    /// once acked. NATS **refuses** to create a second overlapping consumer, so
    /// duplicate processing is impossible rather than merely discouraged.
    ///
    /// Use for work that must happen once (sending an email, charging a card).
    JobQueue,
    /// Event log: independent consumer groups may each read every message, and
    /// messages are retained by age/count regardless of who is currently listening.
    ///
    /// Use for domain facts other components may want to react to later
    /// (`TodoCreated`, `DocumentUploaded`). A group added tomorrow can still replay
    /// what was published today. Costs the server-side exclusivity guarantee, so
    /// correctness within a group relies on every replica sharing one consumer name.
    EventLog,
}

/// Stream configuration trait (type-safe constants).
///
/// # Consumer groups
///
/// [`Self::CONSUMER_NAME`] is a **consumer group**, exactly like Kafka's. JetStream
/// identifies a durable consumer by name, so:
///
/// - replicas sharing a name **compete** — each message is handled once (work queue);
/// - different names each get their **own cursor** — each receives every message (fan-out).
///
/// Fan-out is therefore something you get *between groups*, not between replicas of one
/// worker. Every replica of a given worker must use the same name; that is the default
/// and you should rarely override it. For the rare case where each replica genuinely must
/// see every message, opt in explicitly with
/// [`WorkerConfig::with_per_instance_broadcast`].
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
///     const KIND: StreamKind = StreamKind::JobQueue;
/// }
/// ```
pub trait StreamConfig {
    /// JetStream stream name (e.g., "EMAILS")
    const STREAM_NAME: &'static str;

    /// Consumer group name (e.g., "email-worker"). Shared by every replica.
    const CONSUMER_NAME: &'static str;

    /// Dead letter queue stream name (e.g., "EMAILS_DLQ")
    const DLQ_STREAM: &'static str;

    /// Subject pattern (e.g., "emails.>")
    const SUBJECT: &'static str = ">";

    /// Whether this stream is an exclusive job queue or a multi-subscriber event
    /// log. Defaults to the safer [`StreamKind::JobQueue`]: if a stream is really an
    /// event log, say so deliberately.
    const KIND: StreamKind = StreamKind::JobQueue;

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

    /// Durable consumer name — the *consumer group* this worker joins.
    ///
    /// Every replica of a worker MUST use the same value, otherwise each replica
    /// gets its own cursor and every message is processed once per replica.
    pub consumer_name: String,

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

    /// Whether the stream is an exclusive job queue or a shared event log.
    pub kind: StreamKind,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            stream_name: "JOBS".to_string(),
            consumer_name: "worker".to_string(),
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
            kind: StreamKind::JobQueue,
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

    /// Create from a [`StreamConfig`] trait.
    ///
    /// The consumer name is taken **verbatim** from `S::CONSUMER_NAME`, so every
    /// replica of this worker joins the same consumer group and each message is
    /// handled once. Do not append a per-process suffix here: that silently turns a
    /// work queue into fan-out, and the symptom is duplicated side effects in
    /// production rather than a failure in test.
    pub fn from_stream<S: StreamConfig>() -> Self {
        Self {
            stream_name: S::STREAM_NAME.to_string(),
            consumer_name: S::CONSUMER_NAME.to_string(),
            subject: S::SUBJECT.to_string(),
            dlq_stream: S::DLQ_STREAM.to_string(),
            max_deliver: S::MAX_DELIVER,
            ack_wait: Duration::from_secs(S::ACK_WAIT_SECS),
            kind: S::KIND,
            ..Default::default()
        }
    }

    /// Set the consumer group name.
    pub fn with_consumer_name(mut self, name: impl Into<String>) -> Self {
        self.consumer_name = name.into();
        self
    }

    /// Give **this process** its own consumer, so every replica receives every
    /// message instead of sharing the work.
    ///
    /// This is the rare case — in-memory cache invalidation, config reload,
    /// per-node warmup. Do NOT use it for jobs with side effects: N replicas will
    /// perform the side effect N times.
    ///
    /// Consumers created this way are per-process and accumulate server-side across
    /// restarts; pair with a short `inactive_threshold` if you use it in anger.
    pub fn with_per_instance_broadcast(mut self) -> Self {
        self.consumer_name = format!("{}-{}", self.consumer_name, uuid::Uuid::new_v4());
        self
    }

    /// Set the stream kind (job queue vs event log).
    pub fn with_kind(mut self, kind: StreamKind) -> Self {
        self.kind = kind;
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

    /// The invariant this whole type exists to protect: two processes configured
    /// from the same stream MUST join the same consumer group. If they differ, each
    /// gets its own cursor and every message is handled once *per replica* — which
    /// for the email stream meant one email sent per running pod.
    #[test]
    fn replicas_share_one_consumer_group() {
        let a = WorkerConfig::from_stream::<TestStream>();
        let b = WorkerConfig::from_stream::<TestStream>();
        assert_eq!(
            a.consumer_name, b.consumer_name,
            "replicas must join the same consumer group; a per-process name silently \
             converts a work queue into fan-out"
        );
        assert_eq!(a.consumer_name, TestStream::CONSUMER_NAME);
    }

    #[test]
    fn stream_kind_defaults_to_job_queue() {
        // The safe default: exclusive, exactly-once. An event log must say so.
        assert_eq!(
            WorkerConfig::from_stream::<TestStream>().kind,
            StreamKind::JobQueue
        );
    }

    #[test]
    fn per_instance_broadcast_is_opt_in_and_unique() {
        let a = WorkerConfig::from_stream::<TestStream>().with_per_instance_broadcast();
        let b = WorkerConfig::from_stream::<TestStream>().with_per_instance_broadcast();
        assert_ne!(
            a.consumer_name, b.consumer_name,
            "broadcast needs a consumer per process"
        );
        assert!(a.consumer_name.starts_with(TestStream::CONSUMER_NAME));
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
