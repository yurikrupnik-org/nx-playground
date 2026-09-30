//! Domain models, DTOs and enums for the Todo domain.
//!
//! These types are the single source of truth for the wire contract. They derive
//! `ts_rs::TS` with `#[ts(export)]`, so `cargo test` regenerates the TypeScript
//! DTOs consumed by the SolidJS frontend (`@domain/todo`).

use chrono::{DateTime, Utc};
use sea_orm::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use strum::Display;
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

/// Todo priority levels (maps to the Postgres `todo_priority` enum).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    Default,
    DeriveActiveEnum,
    EnumIter,
    ToSchema,
    TS,
)]
#[ts(export)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "todo_priority")]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum TodoPriority {
    #[sea_orm(string_value = "low")]
    Low,
    /// Default priority
    #[default]
    #[sea_orm(string_value = "medium")]
    Medium,
    #[sea_orm(string_value = "high")]
    High,
}

/// A todo item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Todo {
    /// Unique identifier
    #[ts(as = "String")]
    pub id: Uuid,
    /// Short title
    pub title: String,
    /// Optional longer description
    pub description: String,
    /// Whether the todo is completed
    pub completed: bool,
    /// Priority
    pub priority: TodoPriority,
    /// Creation timestamp (RFC3339)
    #[ts(as = "String")]
    pub created_at: DateTime<Utc>,
    /// Last update timestamp (RFC3339)
    #[ts(as = "String")]
    pub updated_at: DateTime<Utc>,
}

/// DTO for creating a todo.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema, TS)]
#[ts(export)]
pub struct CreateTodo {
    #[validate(length(min = 1, max = 255))]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub priority: TodoPriority,
}

/// DTO for updating a todo. All fields optional (PATCH semantics).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema, TS)]
#[ts(export)]
pub struct UpdateTodo {
    #[validate(length(min = 1, max = 255))]
    pub title: Option<String>,
    pub description: Option<String>,
    pub completed: Option<bool>,
    pub priority: Option<TodoPriority>,
}

/// Query filters for listing todos.
///
/// `Default` matches the serde defaults (`limit` 50, not 0), the same manual
/// impl every sibling domain filter carries — a derived `Default` is `LIMIT 0`.
#[derive(Debug, Clone, Deserialize, ToSchema, IntoParams)]
pub struct TodoFilter {
    pub completed: Option<bool>,
    pub priority: Option<TodoPriority>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

impl Default for TodoFilter {
    fn default() -> Self {
        Self {
            completed: None,
            priority: None,
            limit: default_limit(),
            offset: 0,
        }
    }
}

impl Todo {
    /// Apply a partial update in place, bumping `updated_at`.
    pub fn apply_update(&mut self, update: UpdateTodo) {
        if let Some(title) = update.title {
            self.title = title;
        }
        if let Some(description) = update.description {
            self.description = description;
        }
        if let Some(completed) = update.completed {
            self.completed = completed;
        }
        if let Some(priority) = update.priority {
            self.priority = priority;
        }
        self.updated_at = Utc::now();
    }
}
