use chrono::{DateTime, Utc};
use sea_orm::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use strum::Display;
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

/// Task priority levels
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
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "task_priority")]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum TaskPriority {
    #[sea_orm(string_value = "low")]
    Low,
    /// Default priority
    #[default]
    #[sea_orm(string_value = "medium")]
    Medium,
    #[sea_orm(string_value = "high")]
    High,
    #[sea_orm(string_value = "urgent")]
    Urgent,
}

/// Task status
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
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "task_status")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TaskStatus {
    /// Task not started
    #[default]
    #[sea_orm(string_value = "todo")]
    Todo,
    /// Task in progress
    #[sea_orm(string_value = "in_progress")]
    InProgress,
    /// Task completed
    #[sea_orm(string_value = "done")]
    Done,
}

/// Task entity - represents a task
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Task {
    /// Unique identifier
    #[ts(as = "String")]
    pub id: Uuid,
    /// Owning organization (tenant scope)
    #[ts(as = "String")]
    pub org_id: Uuid,
    /// Creating/owning user within the org
    #[ts(as = "String")]
    pub user_id: Uuid,
    /// Task title
    pub title: String,
    /// Task description
    pub description: String,
    /// Whether the task is completed
    pub completed: bool,
    /// Optional project association
    #[ts(as = "Option<String>")]
    pub project_id: Option<Uuid>,
    /// Task priority
    pub priority: TaskPriority,
    /// Task status
    pub status: TaskStatus,
    /// Optional due date
    #[ts(as = "Option<String>")]
    pub due_date: Option<DateTime<Utc>>,
    /// Creation timestamp
    #[ts(as = "String")]
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    #[ts(as = "String")]
    pub updated_at: DateTime<Utc>,
}

/// DTO for creating a new task
#[derive(Debug, Clone, Deserialize, Validate, ToSchema, TS)]
#[ts(export)]
pub struct CreateTask {
    #[validate(length(min = 1, max = 255))]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[ts(as = "Option<String>")]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub priority: TaskPriority,
    #[serde(default)]
    pub status: TaskStatus,
    #[ts(as = "Option<String>")]
    pub due_date: Option<DateTime<Utc>>,
}

/// DTO for updating an existing task
#[derive(Debug, Clone, Deserialize, Validate, ToSchema, Default, TS)]
#[ts(export)]
pub struct UpdateTask {
    #[validate(length(min = 1, max = 255))]
    pub title: Option<String>,
    pub description: Option<String>,
    pub completed: Option<bool>,
    #[ts(as = "Option<Option<String>>")]
    pub project_id: Option<Option<Uuid>>,
    pub priority: Option<TaskPriority>,
    pub status: Option<TaskStatus>,
    #[ts(as = "Option<Option<String>>")]
    pub due_date: Option<Option<DateTime<Utc>>>,
}

/// Query filters for listing tasks
#[derive(Debug, Clone, Deserialize, ToSchema, IntoParams)]
pub struct TaskFilter {
    pub user_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub completed: Option<bool>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

impl Default for TaskFilter {
    fn default() -> Self {
        Self {
            user_id: None,
            project_id: None,
            status: None,
            priority: None,
            completed: None,
            limit: default_limit(),
            offset: 0,
        }
    }
}

/// Tenant scope resolved from the verified token by the BFF; never client-supplied.
#[derive(Debug, Clone, Copy)]
pub struct TaskScope {
    pub org_id: Uuid,
    pub user_id: Uuid,
}

impl Task {
    /// Apply updates from UpdateTask DTO
    pub fn apply_update(&mut self, update: UpdateTask) {
        if let Some(title) = update.title {
            self.title = title;
        }
        if let Some(description) = update.description {
            self.description = description;
        }
        if let Some(completed) = update.completed {
            self.completed = completed;
        }
        if let Some(project_id) = update.project_id {
            self.project_id = project_id;
        }
        if let Some(priority) = update.priority {
            self.priority = priority;
        }
        if let Some(status) = update.status {
            self.status = status;
        }
        if let Some(due_date) = update.due_date {
            self.due_date = due_date;
        }
        self.updated_at = chrono::Utc::now();
    }
}
