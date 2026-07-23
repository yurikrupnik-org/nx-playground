//! Processor trait for job execution.

use crate::error::ProcessingError;
use crate::job::Job;
use std::future::Future;

/// Job processor trait.
///
/// Implement this trait to define how jobs are processed. The processor
/// is backend-agnostic and works with any queue implementation.
///
/// # Error Handling
///
/// Return a `ProcessingError` with the appropriate category:
/// - `Transient`: Will be retried with exponential backoff
/// - `Permanent`: Will be moved to dead letter queue
/// - `RateLimited`: Will be retried with longer delays
///
/// # Example
///
/// ```rust,ignore
/// use messaging::{Job, Processor, ProcessingError};
///
/// struct EmailProcessor {
///     provider: Arc<dyn EmailProvider>,
///     templates: Arc<TemplateEngine>,
/// }
///
/// impl Processor<EmailJob> for EmailProcessor {
///     async fn process(&self, job: &EmailJob) -> Result<(), ProcessingError> {
///         // Render template
///         let content = self.templates.render(&job.template, &job.data)
///             .map_err(|e| ProcessingError::permanent(e.to_string()))?;
///
///         // Send email
///         self.provider.send(&job.to, &content).await
///             .map_err(|e| {
///                 if e.is_rate_limited() {
///                     ProcessingError::rate_limited(e.to_string())
///                 } else if e.is_temporary() {
///                     ProcessingError::transient(e.to_string())
///                 } else {
///                     ProcessingError::permanent(e.to_string())
///                 }
///             })?;
///
///         Ok(())
///     }
///
///     fn name(&self) -> &'static str {
///         "email_processor"
///     }
///
///     async fn health_check(&self) -> Result<bool, ProcessingError> {
///         self.provider.health_check().await
///             .map_err(|e| ProcessingError::transient(e.to_string()))?;
///         Ok(true)
///     }
/// }
/// ```
pub trait Processor<J: Job>: Send + Sync {
    /// Process a job.
    ///
    /// # Arguments
    ///
    /// * `job` - The job to process
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Job processed successfully
    /// * `Err(ProcessingError)` - Processing failed (retry or DLQ based on category)
    fn process(&self, job: &J) -> impl Future<Output = Result<(), ProcessingError>> + Send;

    /// Get the processor name.
    ///
    /// Used for logging and metrics labels.
    fn name(&self) -> &'static str;

    /// Perform a health check.
    ///
    /// Override to check downstream service availability. This is used
    /// by the worker's readiness probe.
    ///
    /// # Default
    ///
    /// Returns `Ok(true)` (always healthy).
    fn health_check(&self) -> impl Future<Output = Result<bool, ProcessingError>> + Send {
        async { Ok(true) }
    }

    /// Called before processing starts.
    ///
    /// Override for initialization, logging, or metrics.
    fn on_start(&self) -> impl Future<Output = Result<(), ProcessingError>> + Send {
        async { Ok(()) }
    }

    /// Called after processing completes.
    ///
    /// Override for cleanup, logging, or metrics.
    fn on_complete(
        &self,
        _job: &J,
        _result: &Result<(), ProcessingError>,
    ) -> impl Future<Output = ()> + Send {
        async {}
    }
}

/// A no-op processor for testing.
#[derive(Debug, Clone, Default)]
pub struct NoOpProcessor;

impl<J: Job> Processor<J> for NoOpProcessor {
    async fn process(&self, _job: &J) -> Result<(), ProcessingError> {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "noop_processor"
    }
}

/// A processor that always fails (for testing).
#[derive(Debug, Clone)]
pub struct FailingProcessor {
    error_message: String,
    transient: bool,
}

impl FailingProcessor {
    /// Create a processor that fails with transient errors.
    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            error_message: message.into(),
            transient: true,
        }
    }

    /// Create a processor that fails with permanent errors.
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            error_message: message.into(),
            transient: false,
        }
    }
}

impl<J: Job> Processor<J> for FailingProcessor {
    async fn process(&self, _job: &J) -> Result<(), ProcessingError> {
        if self.transient {
            Err(ProcessingError::transient(&self.error_message))
        } else {
            Err(ProcessingError::permanent(&self.error_message))
        }
    }

    fn name(&self) -> &'static str {
        "failing_processor"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Job;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

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

    #[tokio::test]
    async fn test_noop_processor() {
        let processor = NoOpProcessor;
        let job = TestJob {
            id: Uuid::new_v4(),
            retry_count: 0,
        };

        let result = Processor::<TestJob>::process(&processor, &job).await;
        assert!(result.is_ok());
        assert_eq!(Processor::<TestJob>::name(&processor), "noop_processor");
    }

    #[tokio::test]
    async fn test_failing_processor_transient() {
        let processor = FailingProcessor::transient("test failure");
        let job = TestJob {
            id: Uuid::new_v4(),
            retry_count: 0,
        };

        let result = Processor::<TestJob>::process(&processor, &job).await;
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert_eq!(error.category(), crate::error::ErrorCategory::Transient);
    }

    #[tokio::test]
    async fn test_failing_processor_permanent() {
        let processor = FailingProcessor::permanent("test failure");
        let job = TestJob {
            id: Uuid::new_v4(),
            retry_count: 0,
        };

        let result = Processor::<TestJob>::process(&processor, &job).await;
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert_eq!(error.category(), crate::error::ErrorCategory::Permanent);
    }
}
