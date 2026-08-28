//! Integration tests for TODOS event-stream browsing and management.
//!
//! Requires docker (testcontainers NATS with JetStream).

use chrono::Utc;
use domain_todo::{
    NatsTodoPublisher, Todo, TodoEvent, TodoEventKind, TodoEventPublisher, TodoPriority,
};
use test_utils::TestNats;
use todo_cli::{dlq_entries, purge_events, recent_events};
use uuid::Uuid;

fn todo(title: &str) -> Todo {
    let now = Utc::now();
    Todo {
        id: Uuid::now_v7(),
        title: title.to_string(),
        description: String::new(),
        completed: false,
        priority: TodoPriority::Medium,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn browse_filter_limit_and_purge() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();

    // Nothing published yet: absent streams read as empty, not errors.
    assert!(recent_events(&js, None, 10)
        .await
        .expect("empty")
        .is_empty());
    assert!(dlq_entries(&js, 10).await.expect("no dlq").is_empty());
    assert_eq!(purge_events(&js).await.expect("purge absent"), 0);

    let publisher = NatsTodoPublisher::new(js.clone())
        .await
        .expect("init publisher");
    let a = todo("first");
    let b = todo("second");
    for event in [
        TodoEvent::from_todo(TodoEventKind::Created, &a),
        TodoEvent::from_todo(TodoEventKind::Created, &b),
        TodoEvent::from_todo(TodoEventKind::Completed, &a),
        TodoEvent::deleted(b.id),
    ] {
        publisher.publish(event).await.expect("publish");
    }

    // Oldest first, sequences strictly increasing.
    let events = recent_events(&js, None, 10).await.expect("all events");
    assert_eq!(
        events.iter().map(|r| r.event.kind).collect::<Vec<_>>(),
        vec![
            TodoEventKind::Created,
            TodoEventKind::Created,
            TodoEventKind::Completed,
            TodoEventKind::Deleted,
        ]
    );
    assert!(events.windows(2).all(|w| w[0].sequence < w[1].sequence));
    // Deleted events carry no snapshot.
    assert!(events[3].event.todo.is_none());

    // Limit keeps the *last* N.
    let last_two = recent_events(&js, None, 2).await.expect("last two");
    assert_eq!(
        last_two.iter().map(|r| r.event.kind).collect::<Vec<_>>(),
        vec![TodoEventKind::Completed, TodoEventKind::Deleted]
    );

    // Kind filter is a server-side subject filter.
    let created = recent_events(&js, Some(TodoEventKind::Created), 10)
        .await
        .expect("created only");
    assert_eq!(created.len(), 2);
    assert_eq!(
        created.iter().map(|r| r.event.todo_id).collect::<Vec<_>>(),
        vec![a.id, b.id]
    );

    // Purge empties the stream; browsing after purge sees nothing.
    assert_eq!(purge_events(&js).await.expect("purge"), 4);
    assert!(recent_events(&js, None, 10)
        .await
        .expect("after purge")
        .is_empty());
}
