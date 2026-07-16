//! NATS JetStream wiring for the Todo domain: the stream definition and a
//! [`TodoEventPublisher`] backed by the shared `messaging::NatsProducer`.

use async_nats::jetstream::Context;
use async_trait::async_trait;
use messaging::nats::{NatsProducer, StreamConfig};

use crate::error::{TodoError, TodoResult};
use crate::events::{TodoEvent, TodoEventPublisher};

/// JetStream configuration for the todo event stream.
pub struct TodoNatsStream;

impl StreamConfig for TodoNatsStream {
    const STREAM_NAME: &'static str = "TODOS";
    const CONSUMER_NAME: &'static str = "todo-worker";
    const DLQ_STREAM: &'static str = "TODOS_DLQ";
    const SUBJECT: &'static str = "todos.>";
    const MAX_DELIVER: i64 = 5;
}

/// Publishes [`TodoEvent`]s to the `TODOS` JetStream stream.
#[derive(Clone)]
pub struct NatsTodoPublisher {
    producer: NatsProducer,
}

impl NatsTodoPublisher {
    /// Build a publisher, creating the backing `TODOS` stream if it does not
    /// already exist (idempotent). Lets the API publish before any worker runs.
    pub async fn new(jetstream: Context) -> TodoResult<Self> {
        jetstream
            .get_or_create_stream(async_nats::jetstream::stream::Config {
                name: TodoNatsStream::STREAM_NAME.to_string(),
                subjects: vec![TodoNatsStream::SUBJECT.to_string()],
                ..Default::default()
            })
            .await
            .map_err(|e| TodoError::Internal(format!("create TODOS stream: {e}")))?;
        let producer = NatsProducer::from_stream_config::<TodoNatsStream>(jetstream);
        Ok(Self { producer })
    }
}

#[async_trait]
impl TodoEventPublisher for NatsTodoPublisher {
    async fn publish(&self, event: TodoEvent) -> TodoResult<()> {
        let subject = event.subject();
        self.producer
            .send_to(&subject, &event)
            .await
            .map_err(|e| TodoError::Internal(format!("publish todo event: {e}")))?;
        Ok(())
    }
}
