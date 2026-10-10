//! Wire DTOs for todo-api.
//!
//! **Source of truth: `libs/domains/todo/src/models.rs`** (`Todo`, `CreateTodo`,
//! `UpdateTodo`, `TodoPriority`) and `libs/domains/todo/src/events.rs`
//! (`TodoEvent`, `TodoEventKind`). The committed wire shape is
//! `docs/openapi/todos.v1.json`.
//!
//! These are redeclared instead of depended on because `domain_todo` cannot be a
//! dependency of a `wasm32-unknown-unknown` crate at any feature combination: it
//! hard-depends on sea-orm and sqlx (socket-backed drivers), axum (tokio's net
//! stack) and validator/utoipa on top of those. The `@domain/todo` TypeScript
//! DTOs the Solid app imports exist for the same reason — they are ts-rs output
//! from these very types, not a second declaration of the contract.
//!
//! Keep the field names and `rename_all` attributes in sync with that module.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `#[serde(rename_all = "lowercase")]` on the domain enum; also the CSS token
/// (`badge--low`) and the `<option>` value, exactly as in the Solid app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TodoPriority {
    Low,
    Medium,
    High,
}

/// Form/render order, mirroring `PRIORITIES` in `apps/todo/web/src/todo-app.tsx`.
pub const PRIORITIES: [TodoPriority; 3] = [
    TodoPriority::Low,
    TodoPriority::Medium,
    TodoPriority::High,
];

impl TodoPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Parse a `<select>` value; `None` for anything outside [`PRIORITIES`].
    pub fn parse(value: &str) -> Option<Self> {
        PRIORITIES.into_iter().find(|p| p.as_str() == value)
    }
}

impl Default for TodoPriority {
    /// `#[default] Medium` on the domain enum, and the SPA's initial selection.
    fn default() -> Self {
        Self::Medium
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Todo {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub completed: bool,
    pub priority: TodoPriority,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTodo {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub priority: TodoPriority,
}

/// PATCH semantics: every field optional.
///
/// `PUT /api/todos/{id}` is part of the surface `apps/todo/web/src/lib/todo-api.ts`
/// exposes, so the parity mirror carries it — but the `/` route never calls it
/// (a toggle goes through `/complete` and `/uncomplete`), hence `dead_code`.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateTodo {
    pub title: Option<String>,
    pub description: Option<String>,
    pub completed: Option<bool>,
    pub priority: Option<TodoPriority>,
}

/// `#[serde(rename_all = "snake_case")]` on the domain enum. These are also the
/// SSE event NAMES todo-api publishes under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoEventKind {
    Created,
    Updated,
    Completed,
    Uncompleted,
    Deleted,
}

/// Every named SSE event this app subscribes to — `TODO_EVENT_KINDS` in
/// `apps/todo/web/src/lib/realtime.ts`.
pub const TODO_EVENT_KINDS: [&str; 5] = [
    "created",
    "updated",
    "completed",
    "uncompleted",
    "deleted",
];

impl TodoEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Completed => "completed",
            Self::Uncompleted => "uncompleted",
            Self::Deleted => "deleted",
        }
    }
}

/// A database change event, delivered over SSE and over the WebSocket.
/// `todo` is `None` only for `Deleted`.
#[derive(Debug, Clone, Deserialize)]
pub struct TodoEvent {
    pub kind: TodoEventKind,
    pub todo_id: Uuid,
    #[serde(default)]
    pub todo: Option<Todo>,
}
