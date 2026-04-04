//! Dapr subscription handler for receiving pub/sub deliveries.
//!
//! Dapr calls into the app to discover subscriptions (`GET /dapr/subscribe`)
//! and to deliver messages (`POST /{route}`).

use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};

/// A Dapr subscription declaration returned by `GET /dapr/subscribe`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaprSubscription {
    pub pubsubname: String,
    pub topic: String,
    pub route: String,
}

/// Successful response to Dapr after processing a message.
#[derive(Serialize)]
pub struct DaprEventResponse {
    pub status: DaprEventStatus,
}

/// Status codes Dapr understands for event processing results.
#[derive(Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DaprEventStatus {
    /// Message processed successfully.
    Success,
    /// Message should be retried.
    Retry,
    /// Message should be dropped (sent to dead letter topic).
    Drop,
}

impl DaprEventResponse {
    pub fn success() -> Self {
        Self {
            status: DaprEventStatus::Success,
        }
    }

    pub fn retry() -> Self {
        Self {
            status: DaprEventStatus::Retry,
        }
    }

    pub fn drop_message() -> Self {
        Self {
            status: DaprEventStatus::Drop,
        }
    }
}

/// Cloud Events envelope that Dapr wraps around published messages.
#[derive(Debug, Deserialize)]
pub struct CloudEvent<T> {
    /// The event data (the original published payload).
    pub data: T,
    /// Cloud Events spec version.
    #[serde(rename = "specversion")]
    pub spec_version: Option<String>,
    /// Event type.
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    /// Event source.
    pub source: Option<String>,
    /// Unique event ID.
    pub id: Option<String>,
    /// The topic the event was published to.
    pub topic: Option<String>,
    /// The pub/sub component name.
    pub pubsubname: Option<String>,
}

/// Configuration for building subscription routes.
pub struct SubscriptionRoute {
    pub subscriptions: Vec<DaprSubscription>,
}

impl SubscriptionRoute {
    pub fn new() -> Self {
        Self {
            subscriptions: Vec::new(),
        }
    }

    /// Register a subscription that Dapr will discover.
    pub fn subscribe(
        mut self,
        pubsub_name: impl Into<String>,
        topic: impl Into<String>,
        route: impl Into<String>,
    ) -> Self {
        self.subscriptions.push(DaprSubscription {
            pubsubname: pubsub_name.into(),
            topic: topic.into(),
            route: route.into(),
        });
        self
    }

    /// Build the `GET /dapr/subscribe` route that returns all subscriptions.
    pub fn build_discovery_router(self) -> Router {
        let subs = self.subscriptions;
        Router::new().route(
            "/dapr/subscribe",
            get(move || {
                let subs = subs.clone();
                async move { Json(subs) }
            }),
        )
    }
}

impl Default for SubscriptionRoute {
    fn default() -> Self {
        Self::new()
    }
}
