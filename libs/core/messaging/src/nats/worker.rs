//! NATS JetStream worker for processing jobs.
//!
//! IMPROVEMENT: Now processes messages concurrently using a semaphore
//! to respect max_concurrent_jobs configuration.

use crate::nats::config::WorkerConfig;
use crate::nats::consumer::{NatsConsumer, NatsMessage, StreamInfo};
use crate::nats::dlq::DlqManager;
use crate::nats::error::NatsError;
use crate::nats::health::HealthState;
use crate::nats::metrics::NatsMetrics;
use crate::{ErrorCategory, Job, ProcessingError, Processor};
use async_nats::jetstream::Context;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, watch};
use tracing::{debug, error, info, warn};

/// NATS JetStream worker for processing jobs.
pub struct NatsWorker<J: Job, P: Processor<J>> {
    consumer: NatsConsumer,
    dlq: Arc<DlqManager>,
    processor: Arc<P>,
    config: WorkerConfig,
    metrics: Arc<NatsMetrics>,
    health_state: Option<HealthState>,
    _marker: std::marker::PhantomData<J>,
}

impl<J: Job, P: Processor<J> + 'static> NatsWorker<J, P> {
    /// Create a new NATS worker.
    pub async fn new(
        jetstream: Context,
        processor: P,
        config: WorkerConfig,
    ) -> Result<Self, NatsError> {
        let jetstream = Arc::new(jetstream);
        let processor_name = processor.name();

        let consumer = NatsConsumer::new(jetstream.clone(), config.clone());
        let dlq = Arc::new(DlqManager::new(jetstream.clone(), &config.dlq_stream));
        let metrics = Arc::new(NatsMetrics::new(&config.stream_name, processor_name));

        // Initialize stream and consumer
        consumer.init().await?;

        // Initialize DLQ stream
        dlq.ensure_stream().await?;

        Ok(Self {
            consumer,
            dlq,
            processor: Arc::new(processor),
            config,
            metrics,
            health_state: None,
            _marker: std::marker::PhantomData,
        })
    }

    /// Set the health state for K8s probe updates.
    ///
    /// When set, the worker updates `stream_connected` on batch success/failure
    /// so readiness probes reflect actual NATS connectivity.
    pub fn with_health_state(mut self, state: HealthState) -> Self {
        self.health_state = Some(state);
        self
    }

    /// Run the worker loop.
    ///
    /// The worker will:
    /// 1. Fetch messages in batches
    /// 2. Process each message concurrently (up to max_concurrent_jobs)
    /// 3. Ack on success, nak on transient failure, term on permanent failure
    /// 4. Move permanently failed messages to DLQ
    /// 5. Handle shutdown gracefully
    pub async fn run(&self, mut shutdown_rx: watch::Receiver<bool>) -> Result<(), NatsError> {
        info!(
            stream = %self.config.stream_name,
            consumer = %self.config.consumer_name,
            max_concurrent = %self.config.max_concurrent_jobs,
            "Starting NATS worker"
        );

        loop {
            tokio::select! {
                // Check for shutdown
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("Shutdown signal received, stopping worker");
                        break;
                    }
                }

                // Main processing loop
                result = self.process_batch() => {
                    match result {
                        Ok(()) => {
                            if let Some(ref state) = self.health_state {
                                state.set_stream_connected(true).await;
                                state.set_error(None).await;
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "Error processing batch");
                            if let Some(ref state) = self.health_state {
                                state.set_stream_connected(false).await;
                                state.set_error(Some(e.to_string())).await;
                            }
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        }

        info!("NATS worker stopped");
        Ok(())
    }

    /// Process a batch of messages concurrently.
    ///
    /// Uses a semaphore to limit concurrent processing to max_concurrent_jobs.
    async fn process_batch(&self) -> Result<(), NatsError> {
        let fetched = self.consumer.fetch::<J>(self.config.batch_size).await?;

        if fetched.is_empty() {
            // No messages, wait before next poll
            tokio::time::sleep(Duration::from_millis(100)).await;
            self.publish_depth_gauges().await;
            return Ok(());
        }

        // Poison first: these cannot be processed, only captured and terminated.
        for poison in fetched.poison {
            self.metrics.job_received();
            self.metrics.job_failed("poison");

            error!(
                subject = %poison.subject,
                sequence = poison.sequence,
                error = %poison.error,
                bytes = poison.raw.len(),
                "Undeserializable message, moving to DLQ"
            );

            self.dlq
                .move_poison_to_dlq(
                    &poison.raw,
                    &poison.subject,
                    &poison.error,
                    poison.sequence,
                    poison.delivery_count,
                )
                .await?;
            self.metrics.job_moved_to_dlq();
            poison.term().await?;
        }

        // Create a semaphore to limit concurrent processing
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_jobs));
        let mut handles = Vec::with_capacity(fetched.jobs.len());

        for message in fetched.jobs {
            self.metrics.job_received();

            if message.is_redelivery() {
                debug!(
                    job_id = %message.job_id(),
                    sequence = message.sequence,
                    delivery_count = message.delivery_count,
                    "Processing redelivered message"
                );
            }

            // Clone Arcs for the spawned task. `acquire_owned` only fails on a closed
            // semaphore, and this one lives as long as the worker loop below it.
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("concurrency semaphore is never closed while the worker runs");
            let processor = self.processor.clone();
            let dlq = self.dlq.clone();
            let metrics = self.metrics.clone();
            let max_deliver = self.config.max_deliver;

            // Spawn concurrent task
            let handle = tokio::spawn(async move {
                let result = Self::process_message_inner(
                    message,
                    processor.as_ref(),
                    dlq.as_ref(),
                    metrics.as_ref(),
                    max_deliver,
                )
                .await;

                // Release permit when done
                drop(permit);
                result
            });

            handles.push(handle);
        }

        // Wait for all tasks to complete
        for handle in handles {
            if let Err(e) = handle.await {
                error!(error = %e, "Task panicked");
            }
        }

        self.publish_depth_gauges().await;

        Ok(())
    }

    /// Publish stream and DLQ depth gauges.
    ///
    /// The DLQ gauge is the entire operational answer to "who handles the DLQ": a
    /// non-zero `nats_worker_dlq_depth` is an alert, and a human decides whether to
    /// fix and redrive or to discard. There is deliberately no DLQ *consumer* —
    /// a message lands there only after automatic retry is exhausted, so reprocessing
    /// it automatically is the same failure on a slower loop.
    ///
    /// Depth is refreshed once per batch rather than per message: it is two stream
    /// info round-trips, and a gauge only needs to be eventually right.
    async fn publish_depth_gauges(&self) {
        match self.consumer.stream_info().await {
            Ok(info) => self.metrics.stream_depth(info.messages),
            Err(e) => debug!(error = %e, "Could not read stream depth"),
        }
        match self.dlq.stream_info().await {
            Ok(info) => self.metrics.dlq_depth(info.messages),
            // Absent until the first failure creates it; not worth a warning.
            Err(e) => debug!(error = %e, "Could not read DLQ depth"),
        }
    }

    /// Process a single message (static method for use in spawned tasks).
    async fn process_message_inner(
        message: NatsMessage<J>,
        processor: &P,
        dlq: &DlqManager,
        metrics: &NatsMetrics,
        max_deliver: i64,
    ) -> Result<(), NatsError> {
        let job_id = message.job_id();
        let sequence = message.sequence;

        debug!(
            job_id = %job_id,
            sequence = sequence,
            delivery_count = message.delivery_count,
            "Processing job"
        );

        let start = Instant::now();
        let result = processor.process(&message.job).await;
        let duration = start.elapsed();

        match result {
            Ok(()) => {
                // Success - acknowledge
                message.ack().await?;
                metrics.job_processed(duration);

                debug!(
                    job_id = %job_id,
                    sequence = sequence,
                    duration_ms = duration.as_millis(),
                    "Job processed successfully"
                );
            }
            Err(e) => {
                Self::handle_error_inner(message, e, dlq, metrics, max_deliver).await?;
            }
        }

        Ok(())
    }

    /// Handle a processing error (static method for use in spawned tasks).
    ///
    /// The attempt count comes from `message.delivery_count` — the server's counter —
    /// **not** from the payload. A `nak` asks JetStream to redeliver the *stored*
    /// bytes, so a counter the consumer increments is discarded on the way out. The
    /// old code read `job.retry_count()`, which was therefore pinned at 0 forever:
    /// `should_retry` was always true, the DLQ branch below was unreachable, and
    /// backoff never grew past its base delay.
    async fn handle_error_inner(
        message: NatsMessage<J>,
        error: ProcessingError,
        dlq: &DlqManager,
        metrics: &NatsMetrics,
        max_deliver: i64,
    ) -> Result<(), NatsError> {
        let job_id = message.job_id();
        // `delivery_count` is 1 on first delivery, so retries already made is one less.
        let retries_made = message.delivery_count.saturating_sub(1);
        // JetStream stops redelivering after max_deliver attempts and drops the message
        // with no further notice. If this is the last attempt the server will give us,
        // the DLQ decision has to happen now regardless of what the category permits —
        // otherwise a policy that allows more retries than the stream does (RateLimited
        // permits 5; EMAILS sets max_deliver = 5) loses the message silently.
        let last_attempt = max_deliver > 0 && i64::from(message.delivery_count) >= max_deliver;
        let category = error.category();

        metrics.job_failed(category.as_str());

        let give_up = match category {
            ErrorCategory::Permanent => true,
            ErrorCategory::Transient | ErrorCategory::RateLimited => {
                last_attempt || !error.should_retry(retries_made)
            }
        };

        if give_up {
            let reason = if category == ErrorCategory::Permanent {
                "Permanent error, moving to DLQ"
            } else if last_attempt {
                "Final delivery attempt, moving to DLQ"
            } else {
                "Max retries exceeded, moving to DLQ"
            };

            warn!(
                job_id = %job_id,
                error = %error,
                category = category.as_str(),
                delivery_count = message.delivery_count,
                max_deliver = max_deliver,
                "{reason}"
            );

            dlq.move_to_dlq(
                &message.job,
                &message.subject,
                &error.to_string(),
                message.sequence,
                message.delivery_count,
            )
            .await?;

            metrics.job_moved_to_dlq();

            // Terminate (don't redeliver)
            message.term().await?;
        } else {
            // Request redelivery with backoff
            let delay_ms = error.backoff_delay_ms(retries_made);

            warn!(
                job_id = %job_id,
                error = %error,
                delivery_count = message.delivery_count,
                delay_ms = delay_ms,
                "Transient error, will retry"
            );

            metrics.job_retried();

            message
                .nak_with_delay(Duration::from_millis(delay_ms))
                .await?;
        }

        Ok(())
    }

    /// Get stream info.
    pub async fn stream_info(&self) -> Result<StreamInfo, NatsError> {
        self.consumer.stream_info().await
    }

    /// Get DLQ info.
    pub async fn dlq_info(&self) -> Result<StreamInfo, NatsError> {
        self.dlq.stream_info().await
    }
}
