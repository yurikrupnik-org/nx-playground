//! Stream definitions for email processing
//!
//! Provides stream configuration for NATS JetStream.
//!
//! IMPROVEMENT: Removed Redis stream configuration - this is now NATS-only.

use messaging::nats::{StreamConfig as NatsStreamConfig, StreamKind};

/// Email stream configuration for NATS JetStream
///
/// Defines the NATS stream and consumer settings for email processing.
pub struct EmailNatsStream;

impl NatsStreamConfig for EmailNatsStream {
    /// JetStream stream name
    const STREAM_NAME: &'static str = "EMAILS";

    /// Consumer group shared by every email-worker replica, so a queued email is
    /// sent exactly once no matter how many replicas run.
    const CONSUMER_NAME: &'static str = "email-worker";

    /// Dead letter queue stream
    const DLQ_STREAM: &'static str = "EMAILS_DLQ";

    /// Subject pattern for email jobs
    const SUBJECT: &'static str = "emails.>";

    /// Sending an email is a job, not a fact: it must happen exactly once, and no
    /// second consumer group has any business reading it. `JobQueue` retention makes
    /// the server reject an overlapping consumer, so fan-out is impossible rather
    /// than merely discouraged.
    const KIND: StreamKind = StreamKind::JobQueue;

    /// Max delivery attempts before DLQ
    const MAX_DELIVER: i64 = 5;

    /// Ack wait timeout (30 seconds)
    const ACK_WAIT_SECS: u64 = 30;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nats_stream_config() {
        assert_eq!(EmailNatsStream::STREAM_NAME, "EMAILS");
        assert_eq!(EmailNatsStream::CONSUMER_NAME, "email-worker");
        assert_eq!(EmailNatsStream::DLQ_STREAM, "EMAILS_DLQ");
        assert_eq!(EmailNatsStream::SUBJECT, "emails.>");
    }
}
