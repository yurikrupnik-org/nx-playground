//! Job trait for background job processing.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use uuid::Uuid;

/// A job that can be processed by a worker.
///
/// This trait is backend-agnostic and can be used with:
/// - NATS JetStream (`messaging::nats` module)
/// - Any other queue backend
///
/// # Required Methods
///
/// - `job_id`: Unique identifier for the job, stable across redeliveries
///
/// # Why there is no `retry_count`
///
/// A payload-carried retry counter cannot work over JetStream. A `nak` asks the
/// server to redeliver the *stored* message, so any counter the consumer bumps is
/// thrown away — the next delivery carries the original bytes. The authoritative
/// attempt count is `NatsMessage::delivery_count`, which the server maintains.
/// This trait used to expose `retry_count`/`with_retry`/`can_retry`; they were
/// permanently 0, which silently disabled the worker's DLQ path.
///
/// # Example
///
/// ```rust
/// use messaging::Job;
/// use serde::{Serialize, Deserialize};
/// use uuid::Uuid;
///
/// #[derive(Clone, Serialize, Deserialize)]
/// struct EmailJob {
///     id: Uuid,
///     to: String,
///     subject: String,
/// }
///
/// impl Job for EmailJob {
///     fn job_id(&self) -> Uuid {
///         self.id
///     }
/// }
/// ```
pub trait Job: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {
    /// Get the unique job ID.
    ///
    /// This should be a stable identifier that doesn't change across retries.
    fn job_id(&self) -> Uuid;

    /// Get the job priority (default: Normal).
    ///
    /// Higher priority jobs may be processed first, depending on the backend.
    fn priority(&self) -> JobPriority {
        JobPriority::Normal
    }

    /// Get the job type name (for logging, metrics and DLQ entries).
    ///
    /// Default implementation uses the type name.
    fn job_type(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Job priority levels.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum JobPriority {
    /// Low priority - processed last
    Low,
    /// Normal priority (default)
    #[default]
    Normal,
    /// High priority - processed first
    High,
    /// Critical priority - always processed first
    Critical,
}

impl JobPriority {
    /// Get the numeric priority value (higher = more important).
    pub fn value(&self) -> u8 {
        match self {
            JobPriority::Low => 0,
            JobPriority::Normal => 1,
            JobPriority::High => 2,
            JobPriority::Critical => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Serialize, Deserialize)]
    struct TestJob {
        id: Uuid,
    }

    impl Job for TestJob {
        fn job_id(&self) -> Uuid {
            self.id
        }
    }

    #[test]
    fn job_id_is_stable_across_clones() {
        let id = Uuid::new_v4();
        let job = TestJob { id };

        assert_eq!(job.job_id(), id);
        assert_eq!(
            job.clone().job_id(),
            id,
            "redelivery must not change job_id"
        );
    }

    #[test]
    fn job_defaults_are_normal_priority_and_type_name() {
        let job = TestJob { id: Uuid::new_v4() };

        assert_eq!(job.priority(), JobPriority::Normal);
        assert!(
            job.job_type().ends_with("TestJob"),
            "job_type should name the concrete type, got {}",
            job.job_type()
        );
    }

    #[test]
    fn test_job_priority_ordering() {
        assert!(JobPriority::Low < JobPriority::Normal);
        assert!(JobPriority::Normal < JobPriority::High);
        assert!(JobPriority::High < JobPriority::Critical);
    }

    #[test]
    fn test_job_priority_serialization() {
        let priority = JobPriority::High;
        let json = serde_json::to_string(&priority).unwrap();
        assert_eq!(json, "\"high\"");

        let deserialized: JobPriority = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, JobPriority::High);
    }
}
