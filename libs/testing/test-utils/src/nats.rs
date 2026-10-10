//! NATS test infrastructure
//!
//! Provides a `TestNats` helper that creates a NATS container with JetStream for testing.

use async_nats::Client;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::nats::Nats;

// Re-export for test convenience (used by consumers of this crate)
#[allow(unused_imports)]
pub use futures::StreamExt;

/// Test NATS wrapper that ensures proper cleanup
///
/// The container is automatically stopped and removed when this struct is dropped.
/// JetStream is enabled by default for stream-based testing.
///
/// # Example
///
/// ```no_run
/// use test_utils::TestNats;
///
/// # async fn example() {
/// let nats = TestNats::new().await;
///
/// // Get a client for your tests
/// let client = nats.client();
///
/// // Or get JetStream context
/// let jetstream = nats.jetstream();
///
/// // Create streams, publish/consume messages, etc.
/// # }
/// ```
pub struct TestNats {
    #[allow(dead_code)]
    container: ContainerAsync<Nats>,
    client: Client,
    pub connection_string: String,
}

impl TestNats {
    /// Create a new test NATS instance with JetStream enabled
    ///
    /// Uses NATS latest image with JetStream (-js flag).
    pub async fn new() -> Self {
        // Use NATS with JetStream enabled (-js flag)
        let nats_image = Nats::default().with_tag("latest").with_cmd(["-js"]); // Enable JetStream

        let container = nats_image
            .start()
            .await
            .expect("Failed to start NATS container");

        let host_port = container
            .get_host_port_ipv4(4222)
            .await
            .expect("Failed to get NATS port");

        let connection_string = format!("nats://127.0.0.1:{host_port}");

        let client = async_nats::connect(&connection_string)
            .await
            .expect("Failed to connect to NATS");

        tracing::info!(port = host_port, "Test NATS ready with JetStream");

        Self {
            container,
            client,
            connection_string,
        }
    }

    /// Get a cloned client (useful for passing to services)
    pub fn client(&self) -> Client {
        self.client.clone()
    }

    /// Get a JetStream context for stream operations
    pub fn jetstream(&self) -> async_nats::jetstream::Context {
        async_nats::jetstream::new(self.client.clone())
    }

    /// Get the connection string for manual client creation
    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }
}

// Container is automatically cleaned up when TestNats is dropped
impl Drop for TestNats {
    fn drop(&mut self) {
        tracing::debug!("Cleaning up test NATS container");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_nats_connection() {
        let nats = TestNats::new().await;
        let client = nats.client();

        // Test basic pub/sub
        let mut subscriber = client.subscribe("test.subject").await.unwrap();

        client
            .publish("test.subject", "hello".into())
            .await
            .unwrap();
        client.flush().await.unwrap();

        let message = tokio::time::timeout(tokio::time::Duration::from_secs(5), subscriber.next())
            .await
            .expect("Timeout waiting for message")
            .expect("No message received");

        assert_eq!(message.payload.as_ref(), b"hello");
    }

    #[tokio::test]
    async fn test_nats_jetstream() {
        let nats = TestNats::new().await;
        let jetstream = nats.jetstream();

        // Create a stream
        let stream_config = async_nats::jetstream::stream::Config {
            name: "TEST_STREAM".to_string(),
            subjects: vec!["test.>".to_string()],
            ..Default::default()
        };

        jetstream
            .create_stream(stream_config)
            .await
            .expect("Failed to create stream");

        // Publish a message
        let ack = jetstream
            .publish("test.hello", "world".into())
            .await
            .expect("Failed to publish")
            .await
            .expect("Failed to get ack");

        assert!(ack.sequence > 0);

        // Get stream info
        let mut stream = jetstream
            .get_stream("TEST_STREAM")
            .await
            .expect("Failed to get stream");

        let info = stream.info().await.expect("Failed to get stream info");
        assert_eq!(info.state.messages, 1);
    }

    #[tokio::test]
    async fn test_nats_consumer() {
        let nats = TestNats::new().await;
        let jetstream = nats.jetstream();

        // Create stream
        let stream_config = async_nats::jetstream::stream::Config {
            name: "CONSUMER_TEST".to_string(),
            subjects: vec!["consumer.>".to_string()],
            ..Default::default()
        };

        let stream = jetstream
            .create_stream(stream_config)
            .await
            .expect("Failed to create stream");

        // Publish messages
        for i in 0..3 {
            jetstream
                .publish("consumer.test", format!("message-{i}").into())
                .await
                .unwrap()
                .await
                .unwrap();
        }

        // Create consumer
        let consumer = stream
            .create_consumer(async_nats::jetstream::consumer::pull::Config {
                name: Some("test-consumer".to_string()),
                durable_name: Some("test-consumer".to_string()),
                ..Default::default()
            })
            .await
            .expect("Failed to create consumer");

        // Fetch messages
        let mut messages = consumer.fetch().max_messages(10).messages().await.unwrap();

        let mut count = 0;
        while let Some(Ok(msg)) = messages.next().await {
            msg.ack().await.expect("Failed to ack");
            count += 1;
        }

        assert_eq!(count, 3);
    }
}
