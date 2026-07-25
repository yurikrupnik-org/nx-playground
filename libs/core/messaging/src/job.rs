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
/// - `job_id`: Unique identifier for the job
/// - `retry_count`: Current retry count
/// - `with_retry`: Create a copy with incremented retry count
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
///     retry_count: u32,
/// }
///
/// impl Job for EmailJob {
///     fn job_id(&self) -> Uuid {
///         self.id
///     }
///
///     fn retry_count(&self) -> u32 {
///         self.retry_count
///     }
///
///     fn with_retry(&self) -> Self {
///         Self {
///             retry_count: self.retry_count + 1,
///             ..self.clone()
///         }
///     }
/// }
/// ```
pub trait Job: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {
    /// Get the unique job ID.
    ///
    /// This should be a stable identifier that doesn't change across retries.
    fn job_id(&self) -> Uuid;

    /// Get the current retry count.
    ///
    /// Starts at 0 for a new job.
    fn retry_count(&self) -> u32;

    /// Create a new instance with incremented retry count.
    ///
    /// This is called when a job needs to be retried. The implementation
    /// may choose to:
    /// - Keep the same ID (for idempotency tracking)
    /// - Generate a new ID (for unique message IDs)
    fn with_retry(&self) -> Self;

    /// Get the maximum number of retries (default: 3).
    ///
    /// Override this to customize per-job-type retry limits.
    fn max_retries(&self) -> u32 {
        3
    }

    /// Check if the job can be retried.
    fn can_retry(&self) -> bool {
        self.retry_count() < self.max_retries()
    }

    /// Get the job priority (default: Normal).
    ///
    /// Higher priority jobs may be processed first, depending on the backend.
    fn priority(&self) -> JobPriority {
        JobPriority::Normal
    }

    /// Get the job type name (for logging and metrics).
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
        retry_count: u32,
    }

    impl Job for TestJob {
        fn job_id(&self) -> Uuid {
            self.id
        }

        fn retry_count(&self) -> u32 {
            self.retry_count
        }

        fn with_retry(&self) -> Self {
            Self {
                id: self.id,
                retry_count: self.retry_count + 1,
            }
        }
    }

    #[test]
    fn test_job_trait() {
        let id = Uuid::new_v4();
        let job = TestJob { id, retry_count: 0 };

        assert_eq!(job.job_id(), id);
        assert_eq!(job.retry_count(), 0);
        assert!(job.can_retry());
        assert_eq!(job.max_retries(), 3);

        let retried = job.with_retry();
        assert_eq!(retried.retry_count(), 1);
        assert!(retried.can_retry());

        let at_max = TestJob {
            id: Uuid::new_v4(),
            retry_count: 3,
        };
        assert!(!at_max.can_retry());
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
