//! Dapr sidecar client library.
//!
//! Provides typed HTTP clients for interacting with the Dapr sidecar:
//! - **Pub/Sub**: Publish events to NATS JetStream (or any Dapr pub/sub component)
//! - **State Store**: CRUD operations against Dapr-managed state stores (PostgreSQL, MongoDB)
//! - **Subscriptions**: Axum route helpers for receiving Dapr subscription deliveries

pub mod client;
pub mod pubsub;
pub mod state;
pub mod subscription;

pub use client::DaprClient;
pub use pubsub::PubSubClient;
pub use state::StateClient;
pub use subscription::{DaprSubscription, SubscriptionRoute};
