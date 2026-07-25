//! Rust <-> NATS (publish side) integration test.
//!
//! Verifies that `NatsTodoPublisher` ensures the `TODOS` JetStream stream and
//! publishes a `TodoEvent` (as JSON) to the `todos.<kind>` subject, by reading
//! the message back through a pull consumer.
//!
//! Requires Docker. Run: `cargo test -p domain_todo --test events_it`.

use std::time::Duration;

use async_nats::jetstream::consumer::pull::Config as PullConfig;
use chrono::Utc;
use domain_todo::models::{Todo, TodoPriority};
use domain_todo::{NatsTodoPublisher, TodoEvent, TodoEventKind, TodoEventPublisher};
use futures::StreamExt;
use test_utils::TestNats;
use uuid::Uuid;

fn sample_todo() -> Todo {
    Todo {
        id: Uuid::now_v7(),
        title: "write tests".into(),
        description: String::new(),
        completed: false,
        priority: TodoPriority::Medium,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn publishes_event_to_jetstream() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();

    let publisher = NatsTodoPublisher::new(js.clone())
        .await
        .expect("init publisher + stream");

    let todo = sample_todo();
    publisher
        .publish(TodoEvent::from_todo(TodoEventKind::Created, &todo))
        .await
        .expect("publish");

    // Read the message back from the TODOS stream.
    let stream = js.get_stream("TODOS").await.expect("get stream");
    let consumer = stream
        .create_consumer(PullConfig {
            durable_name: Some("test-reader".into()),
            ..Default::default()
        })
        .await
        .expect("create consumer");

    let mut messages = consumer
        .fetch()
        .max_messages(1)
        .messages()
        .await
        .expect("fetch");

    let msg = tokio::time::timeout(Duration::from_secs(5), messages.next())
        .await
        .expect("timed out waiting for message")
        .expect("stream ended")
        .expect("message error");

    assert_eq!(msg.subject.as_str(), "todos.created");
    let event: TodoEvent = serde_json::from_slice(&msg.payload).expect("decode event");
    assert_eq!(event.kind, TodoEventKind::Created);
    assert_eq!(event.todo_id, todo.id);
    assert_eq!(event.todo.expect("snapshot").title, "write tests");

    msg.ack().await.expect("ack");
}
