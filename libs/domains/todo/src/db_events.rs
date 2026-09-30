//! Database-driven change events: Postgres `LISTEN/NOTIFY` → [`TodoEvent`].
//!
//! The `todos_notify` trigger (migration `20260828133022_todo_notify`) emits a
//! `NOTIFY todo_events` on every committed insert/update/delete. [`listen`]
//! translates those into the same [`TodoEvent`]s the service publishes and pushes
//! them onto a broadcast channel, which the API fans out to browsers over
//! SSE/WebSocket.
//!
//! Why the database and not the service: a service-level tee only sees writes made
//! by *its own process*. Anything else — a second API replica, `todo-worker`, the
//! todo CLI, a migration, a hand-run `psql` UPDATE — moved the data with no UI
//! noticing. Sourcing events from the DB makes the UI correct for every writer.
//!
//! The trigger payload carries only `{kind, id}` (NOTIFY is capped at 8000 bytes
//! and `description` is unbounded TEXT), so this module hydrates the row snapshot
//! itself. That read doubles as cache maintenance: the entry is invalidated first,
//! so the snapshot — and the cache behind it — reflect the new committed state.

use serde::Deserialize;
use sqlx::postgres::{PgListener, PgPool};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::cache::CachedTodoRepository;
use crate::error::{TodoError, TodoResult};
use crate::events::{TodoEvent, TodoEventKind};
use crate::repository::TodoRepository;

/// Postgres channel the `todos_notify` trigger publishes on.
pub const CHANNEL: &str = "todo_events";

/// A decoded `NOTIFY` payload: which row changed and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct TodoNotification {
    pub kind: TodoEventKind,
    pub id: Uuid,
}

/// Decode a trigger payload. Malformed payloads are reported, never panicked on:
/// a bad notification must not kill the listener.
pub fn parse_notification(payload: &str) -> TodoResult<TodoNotification> {
    serde_json::from_str(payload)
        .map_err(|e| TodoError::Internal(format!("bad todo_events payload {payload:?}: {e}")))
}

/// Turn a notification into an event, hydrating the row for every kind but
/// `Deleted` (where the row is already gone and only the id is knowable).
///
/// Invalidates the cached view of the row first so the snapshot cannot be served
/// from a cache that predates the change.
async fn build_event<R: TodoRepository>(
    repo: &CachedTodoRepository<R>,
    notification: TodoNotification,
) -> TodoResult<TodoEvent> {
    repo.invalidate(notification.id).await;

    if notification.kind == TodoEventKind::Deleted {
        return Ok(TodoEvent::deleted(notification.id));
    }

    match repo.get_by_id(notification.id).await? {
        Some(todo) => Ok(TodoEvent::from_todo(notification.kind, &todo)),
        // Raced with a delete that committed after this notification: report the
        // row as gone rather than inventing a snapshot.
        None => Ok(TodoEvent::deleted(notification.id)),
    }
}

/// Open a dedicated connection and `LISTEN` on [`CHANNEL`].
///
/// Separate from [`pump`] so a caller can fail fast on a bad connection before
/// spawning, and so tests can guarantee the subscription exists before they write.
pub async fn subscribe(pool: &PgPool) -> TodoResult<PgListener> {
    let mut listener = PgListener::connect_with(pool)
        .await
        .map_err(|e| TodoError::Internal(format!("todo_events listener connect failed: {e}")))?;
    listener
        .listen(CHANNEL)
        .await
        .map_err(|e| TodoError::Internal(format!("LISTEN {CHANNEL} failed: {e}")))?;

    info!(channel = CHANNEL, "listening for database todo changes");
    Ok(listener)
}

/// Forward notifications onto `events` until the connection dies.
///
/// Runs forever; spawn it. `PgListener::recv` reconnects and re-subscribes on
/// connection loss, so a database restart is survivable — but notifications that
/// occurred while disconnected are lost by design (NOTIFY has no backlog), which
/// is why clients refetch on reconnect.
pub async fn pump<R: TodoRepository>(
    mut listener: PgListener,
    repo: &CachedTodoRepository<R>,
    events: broadcast::Sender<TodoEvent>,
) -> TodoResult<()> {
    loop {
        let notification = match listener.recv().await {
            Ok(n) => n,
            Err(e) => {
                return Err(TodoError::Internal(format!(
                    "todo_events listener stopped: {e}"
                )));
            }
        };

        let decoded = match parse_notification(notification.payload()) {
            Ok(decoded) => decoded,
            Err(e) => {
                warn!(error = %e, "ignoring malformed todo notification");
                continue;
            }
        };

        match build_event(repo, decoded).await {
            Ok(event) => {
                // Err means "no subscribers", which is normal with no browsers open.
                let delivered = events.send(event).unwrap_or(0);
                debug!(
                    kind = ?decoded.kind,
                    id = %decoded.id,
                    subscribers = delivered,
                    "database change fanned out"
                );
            }
            Err(e) => error!(error = %e, id = %decoded.id, "failed to hydrate changed todo"),
        }
    }
}

/// [`subscribe`] then [`pump`]: the whole listener as one future to spawn.
pub async fn listen<R: TodoRepository>(
    pool: &PgPool,
    repo: &CachedTodoRepository<R>,
    events: broadcast::Sender<TodoEvent>,
) -> TodoResult<()> {
    let listener = subscribe(pool).await?;
    pump(listener, repo, events).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Todo, TodoPriority};
    use crate::repository::MockTodoRepository;
    use chrono::Utc;

    fn todo(id: Uuid, completed: bool) -> Todo {
        Todo {
            id,
            title: "t".into(),
            description: String::new(),
            completed,
            priority: TodoPriority::Medium,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn decodes_every_kind_the_trigger_emits() {
        let id = Uuid::now_v7();
        for (raw, expected) in [
            ("created", TodoEventKind::Created),
            ("updated", TodoEventKind::Updated),
            ("completed", TodoEventKind::Completed),
            ("uncompleted", TodoEventKind::Uncompleted),
            ("deleted", TodoEventKind::Deleted),
        ] {
            let payload = format!(r#"{{"kind":"{raw}","id":"{id}"}}"#);
            let decoded = parse_notification(&payload).expect("payload decodes");
            assert_eq!(decoded, TodoNotification { kind: expected, id });
        }
    }

    #[test]
    fn malformed_payloads_are_errors_not_panics() {
        for payload in ["", "{}", "not json", r#"{"kind":"exploded","id":"x"}"#] {
            assert!(
                parse_notification(payload).is_err(),
                "expected {payload:?} to be rejected"
            );
        }
    }

    #[tokio::test]
    async fn hydrates_a_snapshot_for_non_delete_kinds() {
        let id = Uuid::now_v7();
        let mut repo = MockTodoRepository::new();
        repo.expect_get_by_id()
            .withf(move |arg| *arg == id)
            .returning(move |_| Ok(Some(todo(id, true))));
        let repo = CachedTodoRepository::passthrough(repo);

        let event = build_event(
            &repo,
            TodoNotification {
                kind: TodoEventKind::Completed,
                id,
            },
        )
        .await
        .expect("event built");

        assert_eq!(event.kind, TodoEventKind::Completed);
        assert_eq!(event.todo_id, id);
        assert_eq!(
            event.todo.map(|t| t.completed),
            Some(true),
            "the snapshot must come from the database, not the notification"
        );
    }

    #[tokio::test]
    async fn delete_needs_no_row_read() {
        let id = Uuid::now_v7();
        let mut repo = MockTodoRepository::new();
        repo.expect_get_by_id()
            .never()
            .returning(|_| Ok(Some(todo(Uuid::now_v7(), false))));
        let repo = CachedTodoRepository::passthrough(repo);

        let event = build_event(
            &repo,
            TodoNotification {
                kind: TodoEventKind::Deleted,
                id,
            },
        )
        .await
        .expect("event built");

        assert_eq!(event.kind, TodoEventKind::Deleted);
        assert_eq!(event.todo_id, id);
        assert!(event.todo.is_none());
    }

    #[tokio::test]
    async fn a_row_deleted_mid_flight_degrades_to_deleted() {
        let id = Uuid::now_v7();
        let mut repo = MockTodoRepository::new();
        repo.expect_get_by_id().returning(|_| Ok(None));
        let repo = CachedTodoRepository::passthrough(repo);

        let event = build_event(
            &repo,
            TodoNotification {
                kind: TodoEventKind::Updated,
                id,
            },
        )
        .await
        .expect("event built");

        assert_eq!(event.kind, TodoEventKind::Deleted);
        assert!(event.todo.is_none());
    }
}
