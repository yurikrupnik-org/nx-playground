//! Rust <-> NATS (consume side) integration test.
//!
//! Publishes a `TodoEvent` through `NatsTodoPublisher` (which creates the
//! `TODOS` stream), runs the real `NatsWorker` with `TodoProcessor`, and asserts
//! the event was consumed and projected. End-to-end publisher -> JetStream ->
//! worker round trip.
//!
//! Requires Docker. Run: `cargo test -p todo_worker --test consume_it`.

use std::time::{Duration, Instant};

use chrono::Utc;
use domain_todo::models::{Todo, TodoPriority};
use domain_todo::{
    NatsTodoPublisher, TodoEvent, TodoEventKind, TodoEventPublisher, TodoNatsStream,
};
use messaging::nats::{NatsWorker, WorkerConfig};
use test_utils::TestNats;
use todo_worker::TodoProcessor;
use uuid::Uuid;

fn sample_todo() -> Todo {
    Todo {
        id: Uuid::now_v7(),
        title: "ship it".into(),
        description: String::new(),
        completed: true,
        priority: TodoPriority::High,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn worker_consumes_and_projects_events() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();

    // Publisher creates the TODOS stream and publishes a completed event.
    let publisher = NatsTodoPublisher::new(js.clone())
        .await
        .expect("init publisher + stream");
    let todo = sample_todo();
    publisher
        .publish(TodoEvent::from_todo(TodoEventKind::Completed, &todo))
        .await
        .expect("publish");

    // Run the worker (shares Arc counters with our handle via Clone).
    let processor = TodoProcessor::new();
    let observer = processor.clone();
    let config = WorkerConfig::from_stream::<TodoNatsStream>();
    let worker = NatsWorker::<TodoEvent, _>::new(js.clone(), processor, config)
        .await
        .expect("create worker");

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(async move { worker.run(shutdown_rx).await });

    // Wait for the event to be processed.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if observer.processed_count() >= 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not process event within timeout"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert_eq!(observer.completed_count(), 1, "completed projection");

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}
