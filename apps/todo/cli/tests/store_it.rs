//! Integration tests for the KV-backed todo store and status reporting.
//!
//! Requires docker (testcontainers NATS with JetStream).

use domain_todo::{
    CreateTodo, NatsTodoPublisher, TodoEvent, TodoEventKind, TodoEventPublisher, TodoPriority,
};
use test_utils::TestNats;
use todo_cli::{status, TodoStore};

fn create(title: &str, priority: TodoPriority) -> CreateTodo {
    CreateTodo {
        title: title.to_string(),
        description: String::new(),
        priority,
    }
}

#[tokio::test]
async fn roundtrip_updates_store_and_status() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();
    let store = TodoStore::open(&js).await.expect("open bucket");

    // Empty store, no events yet.
    let report = status(&js, &store).await.expect("status");
    assert_eq!(report.total, 0);
    assert_eq!(report.stream_messages, None);

    let first = store
        .add(create("write cli", TodoPriority::High))
        .await
        .expect("add first");
    let second = store
        .add(create("ship it", TodoPriority::Medium))
        .await
        .expect("add second");

    // Oldest first (uuid v7 ids are time-ordered alongside created_at).
    let todos = store.list().await.expect("list");
    assert_eq!(
        todos.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![first.id, second.id]
    );

    // The short id shown by `list` is the uuid tail (random bits); the head
    // is a v7 timestamp shared by ids minted together, hence ambiguous.
    let first_id = first.id.to_string();
    let resolved = store
        .resolve(&first_id[first_id.len() - 8..])
        .await
        .expect("resolve by short id");
    assert_eq!(resolved.id, first.id);
    let err = store
        .resolve(&first_id[..8])
        .await
        .expect_err("timestamp prefix shared by both ids");
    assert!(err.to_string().contains("ambiguous"));
    let err = store.resolve("no-such-id").await.expect_err("no match");
    assert!(err.to_string().contains("no todo matches"));

    // Complete one and publish the event the worker would consume.
    let mut done = resolved;
    done.completed = true;
    store.put(&done).await.expect("put completed");
    let publisher = NatsTodoPublisher::new(js.clone())
        .await
        .expect("init publisher");
    publisher
        .publish(TodoEvent::from_todo(TodoEventKind::Completed, &done))
        .await
        .expect("publish completed event");

    let report = status(&js, &store).await.expect("status after complete");
    assert_eq!(report.total, 2);
    assert_eq!(report.completed, 1);
    assert_eq!(report.pending(), 1);
    assert_eq!((report.high, report.medium, report.low), (1, 1, 0));
    assert_eq!(report.stream_messages, Some(1));
    assert_eq!(report.dlq_messages, None);

    // Remove drops it from the store.
    store.remove(second.id).await.expect("remove");
    let todos = store.list().await.expect("list after remove");
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0].id, first.id);
}

#[tokio::test]
async fn add_rejects_empty_title() {
    let nats = TestNats::new().await;
    let store = TodoStore::open(&nats.jetstream())
        .await
        .expect("open bucket");
    let err = store
        .add(create("", TodoPriority::Low))
        .await
        .expect_err("empty title rejected");
    assert!(err.to_string().contains("title"));
}
