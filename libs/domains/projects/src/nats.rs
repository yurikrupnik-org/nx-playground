//! NATS JetStream publisher for project events.
//!
//! Stream identity comes from `contract_projects` so the publisher and the
//! consuming service cannot drift on the stream name, subject space or
//! retention.

use async_nats::jetstream::Context;
use async_trait::async_trait;
use contract_projects::{PROJECTS_KIND, PROJECTS_STREAM, PROJECTS_SUBJECT, ProjectDeleted};
use messaging::nats::{NatsProducer, stream_config_for};

use crate::error::{ProjectError, ProjectResult};
use crate::events::ProjectEventPublisher;

/// Publishes project events to the `PROJECTS` JetStream stream.
#[derive(Clone)]
pub struct NatsProjectPublisher {
    producer: NatsProducer,
}

impl NatsProjectPublisher {
    /// Build a publisher, creating the backing `PROJECTS` stream if absent
    /// (idempotent), so the API can publish before any consumer runs.
    pub async fn new(jetstream: Context) -> ProjectResult<Self> {
        jetstream
            .get_or_create_stream(stream_config_for(
                PROJECTS_STREAM,
                PROJECTS_SUBJECT,
                PROJECTS_KIND,
            ))
            .await
            .map_err(|e| ProjectError::Internal(format!("create PROJECTS stream: {e}")))?;
        Ok(Self {
            producer: NatsProducer::new(jetstream, PROJECTS_STREAM, PROJECTS_SUBJECT),
        })
    }
}

#[async_trait]
impl ProjectEventPublisher for NatsProjectPublisher {
    async fn publish_deleted(&self, event: ProjectDeleted) -> ProjectResult<()> {
        self.producer
            .send_to(ProjectDeleted::SUBJECT, &event)
            .await
            .map_err(|e| ProjectError::Internal(format!("publish project deleted: {e}")))?;
        Ok(())
    }
}
