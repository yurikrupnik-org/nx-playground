//! Dapr pub/sub publish helper.

use crate::client::DaprClient;
use eyre::{Context, Result};
use serde::Serialize;
use tracing::{debug, instrument};

/// Client for publishing events via Dapr pub/sub.
#[derive(Clone)]
pub struct PubSubClient {
    dapr: DaprClient,
    pubsub_name: String,
}

impl PubSubClient {
    /// Create a new pub/sub client for the given Dapr pub/sub component.
    pub fn new(dapr: DaprClient, pubsub_name: impl Into<String>) -> Self {
        Self {
            dapr,
            pubsub_name: pubsub_name.into(),
        }
    }

    /// Publish an event to a topic.
    ///
    /// Uses `POST /v1.0/publish/{pubsubname}/{topic}`.
    #[instrument(skip(self, event), fields(pubsub = %self.pubsub_name, topic))]
    pub async fn publish<T: Serialize>(&self, topic: &str, event: &T) -> Result<()> {
        let path = format!("/v1.0/publish/{}/{}", self.pubsub_name, topic);
        let resp = self
            .dapr
            .post(&path, event)
            .await
            .wrap_err("Failed to publish event to Dapr pub/sub")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(eyre::eyre!(
                "Dapr publish failed: status={}, body={}",
                status,
                body
            ));
        }

        debug!(topic, "Event published successfully");
        Ok(())
    }

    /// Publish an event with metadata headers.
    #[instrument(skip(self, event, metadata), fields(pubsub = %self.pubsub_name, topic))]
    pub async fn publish_with_metadata<T: Serialize>(
        &self,
        topic: &str,
        event: &T,
        metadata: &std::collections::HashMap<String, String>,
    ) -> Result<()> {
        let path = format!(
            "/v1.0/publish/{}/{}?{}",
            self.pubsub_name,
            topic,
            metadata
                .iter()
                .map(|(k, v)| format!("metadata.{}={}", k, v))
                .collect::<Vec<_>>()
                .join("&")
        );
        let resp = self
            .dapr
            .post(&path, event)
            .await
            .wrap_err("Failed to publish event with metadata")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(eyre::eyre!(
                "Dapr publish with metadata failed: status={}, body={}",
                status,
                body
            ));
        }

        debug!(topic, "Event published with metadata");
        Ok(())
    }
}
