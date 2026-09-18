//! Todo lifecycle events published to NATS JetStream.
//!
//! `TodoEvent` implements `messaging::Job`, so it rides the same JetStream
//! producer/worker machinery as the email jobs. Each event is published to
//! `todos.<kind>` within the `todos.>` subject space.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::error::TodoResult;
use crate::models::Todo;

/// The kind of lifecycle transition an event represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum TodoEventKind {
    Created,
    Updated,
    Completed,
    Uncompleted,
    Deleted,
}

impl TodoEventKind {
    /// Subject suffix used under the `todos.` prefix.
    pub fn subject_suffix(self) -> &'static str {
        match self {
            TodoEventKind::Created => "created",
            TodoEventKind::Updated => "updated",
            TodoEventKind::Completed => "completed",
            TodoEventKind::Uncompleted => "uncompleted",
            TodoEventKind::Deleted => "deleted",
        }
    }
}

/// A todo lifecycle event. `todo` carries a snapshot for all kinds except
/// `Deleted` (where only the id is known).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TodoEvent {
    #[ts(as = "String")]
    pub event_id: Uuid,
    pub kind: TodoEventKind,
    #[ts(as = "String")]
    pub todo_id: Uuid,
    pub todo: Option<Todo>,
    #[ts(as = "String")]
    pub occurred_at: DateTime<Utc>,
}

impl TodoEvent {
    /// Build an event carrying a todo snapshot.
    pub fn from_todo(kind: TodoEventKind, todo: &Todo) -> Self {
        Self {
            event_id: Uuid::now_v7(),
            kind,
            todo_id: todo.id,
            todo: Some(todo.clone()),
            occurred_at: Utc::now(),
        }
    }

    /// Build a `Deleted` event from an id only.
    pub fn deleted(todo_id: Uuid) -> Self {
        Self {
            event_id: Uuid::now_v7(),
            kind: TodoEventKind::Deleted,
            todo_id,
            todo: None,
            occurred_at: Utc::now(),
        }
    }

    /// Concrete subject, e.g. `todos.created`.
    pub fn subject(&self) -> String {
        format!("todos.{}", self.kind.subject_suffix())
    }
}

impl messaging::Job for TodoEvent {
    fn job_id(&self) -> uuid::Uuid {
        self.event_id
    }

    fn job_type(&self) -> &'static str {
        "todo_event"
    }
}

/// Publishes todo events. Abstracted so the service stays transport-agnostic
/// and testable (mockable) without a live NATS connection.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TodoEventPublisher: Send + Sync {
    async fn publish(&self, event: TodoEvent) -> TodoResult<()>;
}

/// No-op publisher for binaries/tests that run without events.
#[derive(Clone, Default)]
pub struct NoopTodoPublisher;

#[async_trait]
impl TodoEventPublisher for NoopTodoPublisher {
    async fn publish(&self, _event: TodoEvent) -> TodoResult<()> {
        Ok(())
    }
}
