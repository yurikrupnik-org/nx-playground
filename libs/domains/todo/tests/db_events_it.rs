//! End-to-end proof that the database drives realtime UI updates.
//!
//! Every write here goes through **raw SQL on the pool**, never through
//! `TodoService`/`PgTodoRepository`. That is the whole point: it stands in for the
//! writers a per-process event tee cannot see (a second API replica, `todo-worker`,
//! the todo CLI, an operator running `psql`). If the `todos_notify` trigger or the
//! listener regresses, these tests fail while the API's own tests stay green.
//!
//! Requires Docker. Run: `cargo test -p domain_todo --test db_events_it`.

use std::sync::Arc;
use std::time::Duration;

use domain_todo::db_events;
use domain_todo::{CachedTodoRepository, PgTodoRepository, TodoEvent, TodoEventKind};
use test_utils::TestDatabase;
use tokio::sync::broadcast;
use tokio::time::timeout;
use uuid::Uuid;

/// Longest a committed change may take to reach a subscriber.
const DELIVERY: Duration = Duration::from_secs(10);

struct Harness {
    _db: TestDatabase,
    pool: sqlx::PgPool,
    events: broadcast::Sender<TodoEvent>,
}

impl Harness {
    /// Boot Postgres with the todo migrations (trigger included) and start the
    /// listener, with `LISTEN` guaranteed established before returning.
    async fn start() -> Self {
        let db = TestDatabase::with_migrations_dir("manifests/db/todo/migrations").await;
        let connection = db.connection();
        let pool = connection.get_postgres_connection_pool().clone();

        let repo = Arc::new(CachedTodoRepository::passthrough(PgTodoRepository::new(
            connection,
        )));
        let (events, _) = broadcast::channel(64);

        // subscribe() before spawning: no race between the writes below and LISTEN.
        let listener = db_events::subscribe(&pool)
            .await
            .expect("LISTEN todo_events");
        let pump_events = events.clone();
        tokio::spawn(async move {
            let _ = db_events::pump(listener, &*repo, pump_events).await;
        });

        Self {
            _db: db,
            pool,
            events,
        }
    }

    async fn insert(&self, id: Uuid, title: &str, description: &str) {
        sqlx::query(
            "INSERT INTO todos (id, title, description, priority) \
             VALUES ($1, $2, $3, 'high'::todo_priority)",
        )
        .bind(id)
        .bind(title)
        .bind(description)
        .execute(&self.pool)
        .await
        .expect("sql insert");
    }

    async fn set_completed(&self, id: Uuid, completed: bool) {
        sqlx::query("UPDATE todos SET completed = $2 WHERE id = $1")
            .bind(id)
            .bind(completed)
            .execute(&self.pool)
            .await
            .expect("sql update completed");
    }

    async fn rename(&self, id: Uuid, title: &str) {
        sqlx::query("UPDATE todos SET title = $2 WHERE id = $1")
            .bind(id)
            .bind(title)
            .execute(&self.pool)
            .await
            .expect("sql update title");
    }

    async fn delete(&self, id: Uuid) {
        sqlx::query("DELETE FROM todos WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .expect("sql delete");
    }

    async fn next_event(rx: &mut broadcast::Receiver<TodoEvent>) -> TodoEvent {
        timeout(DELIVERY, rx.recv())
            .await
            .expect("a database change should reach subscribers")
            .expect("broadcast channel stays open")
    }
}

#[tokio::test]
async fn sql_writes_reach_subscribers_as_lifecycle_events() {
    let h = Harness::start().await;
    let mut rx = h.events.subscribe();
    let id = Uuid::now_v7();

    // --- INSERT -> created, with a snapshot hydrated from the row ---
    h.insert(id, "written in sql", "no api involved").await;

    let event = Harness::next_event(&mut rx).await;
    assert_eq!(event.kind, TodoEventKind::Created);
    assert_eq!(event.todo_id, id);
    let todo = event.todo.expect("created carries a snapshot");
    assert_eq!(todo.title, "written in sql");
    assert_eq!(
        todo.description, "no api involved",
        "the snapshot is read from the database, not from the notification payload"
    );

    // --- completed flag flips are their own kinds, not generic updates ---
    h.set_completed(id, true).await;
    let event = Harness::next_event(&mut rx).await;
    assert_eq!(event.kind, TodoEventKind::Completed);
    assert_eq!(
        event.todo.map(|t| t.completed),
        Some(true),
        "snapshot must reflect the committed value"
    );

    h.set_completed(id, false).await;
    assert_eq!(
        Harness::next_event(&mut rx).await.kind,
        TodoEventKind::Uncompleted
    );

    // --- any other column change is a plain update ---
    h.rename(id, "renamed in sql").await;
    let event = Harness::next_event(&mut rx).await;
    assert_eq!(event.kind, TodoEventKind::Updated);
    assert_eq!(
        event.todo.map(|t| t.title),
        Some("renamed in sql".to_string())
    );

    // --- DELETE -> deleted, id only (the row is gone) ---
    h.delete(id).await;
    let event = Harness::next_event(&mut rx).await;
    assert_eq!(event.kind, TodoEventKind::Deleted);
    assert_eq!(event.todo_id, id);
    assert!(event.todo.is_none());
}

#[tokio::test]
async fn a_rolled_back_change_is_never_published() {
    let h = Harness::start().await;
    let mut rx = h.events.subscribe();
    let rolled_back = Uuid::now_v7();
    let committed = Uuid::now_v7();

    // NOTIFY is transactional, so an aborted write must not reach any UI.
    let mut tx = h.pool.begin().await.expect("begin");
    sqlx::query("INSERT INTO todos (id, title) VALUES ($1, 'phantom')")
        .bind(rolled_back)
        .execute(&mut *tx)
        .await
        .expect("insert inside transaction");
    tx.rollback().await.expect("rollback");

    // Commit a second row: if the phantom had been published it would arrive first.
    h.insert(committed, "real", "").await;

    let event = Harness::next_event(&mut rx).await;
    assert_eq!(
        event.todo_id, committed,
        "the rolled-back insert must never be published"
    );
}

#[tokio::test]
async fn batch_writes_produce_one_event_per_row() {
    let h = Harness::start().await;
    let mut rx = h.events.subscribe();
    let ids: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
    let titles: Vec<String> = (0..3).map(|i| format!("batch {i}")).collect();

    // One statement, three rows.
    sqlx::query("INSERT INTO todos (id, title) SELECT * FROM UNNEST($1::uuid[], $2::varchar[])")
        .bind(&ids)
        .bind(&titles)
        .execute(&h.pool)
        .await
        .expect("batch insert");

    // Row-level trigger: three rows in one statement means three events.
    let mut seen = Vec::new();
    for _ in 0..ids.len() {
        let event = Harness::next_event(&mut rx).await;
        assert_eq!(event.kind, TodoEventKind::Created);
        seen.push(event.todo_id);
    }
    seen.sort();
    let mut expected = ids.clone();
    expected.sort();
    assert_eq!(seen, expected);
}
