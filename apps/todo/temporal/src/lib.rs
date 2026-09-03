//! Todo lifecycle on Temporal — the workflow-as-entity counterpart to
//! `todo_api` + `todo_worker`.
//!
//! One [`TodoWorkflow`] execution *is* one todo: state lives in the workflow
//! (durably persisted by Temporal's event history, no Postgres row), reads go
//! through a query, and mutations arrive as signals. Side effects — publishing
//! [`TodoEvent`]s to the same `todos.>` JetStream subjects the existing
//! `todo_worker` consumes — are confined to activities, the only place
//! nondeterminism is allowed.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use domain_todo::{CreateTodo, Todo, TodoEvent, TodoEventKind, TodoEventPublisher, UpdateTodo};
use serde::{Deserialize, Serialize};
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::{
    ActivityOptions, SyncWorkflowContext, WorkflowContext, WorkflowContextView, WorkflowResult,
    activities::{ActivityContext, ActivityError},
};
use uuid::Uuid;

/// Task queue shared by the worker and the starter.
pub const TASK_QUEUE: &str = "todo-lifecycle";

/// Workflow ids are `todo-<uuid>`, so the todo id doubles as the durable
/// entity address (signals/queries need nothing but the todo id).
pub const WORKFLOW_ID_PREFIX: &str = "todo-";

/// Input for [`TodoActivities::publish_event`]. The workflow ships the kind and
/// snapshot; the activity stamps `event_id`/`occurred_at` (wall clock is fine
/// there — activity results are recorded, never replayed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishTodoEvent {
    pub kind: TodoEventKind,
    pub todo: Todo,
}

/// One execution per todo. `Default` is the pre-`run` state.
#[workflow]
#[derive(Default)]
pub struct TodoWorkflow {
    todo: Option<Todo>,
    deleted: bool,
}

#[workflow_methods]
impl TodoWorkflow {
    /// Lifecycle: create → (update* | complete | delete) → publish final event.
    ///
    /// Returns the final snapshot, or `None` when deleted. The wait in the
    /// middle is durable — the entity survives worker restarts and can idle
    /// for months without holding any process resources.
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        input: CreateTodo,
    ) -> WorkflowResult<Option<Todo>> {
        let now = workflow_now(ctx.workflow_time());
        let todo = Todo {
            // Derive the id from the workflow id so callers address the entity
            // by todo id alone; fall back to a replay-safe SDK uuid.
            id: todo_id(ctx.workflow_id())
                .unwrap_or_else(|| Uuid::parse_str(&ctx.uuid4()).expect("sdk uuid4 is valid")),
            title: input.title,
            description: input.description,
            completed: false,
            priority: input.priority,
            created_at: now,
            updated_at: now,
        };
        ctx.state_mut(|s| s.todo = Some(todo.clone()));

        ctx.execute_activity(
            TodoActivities::publish_event,
            PublishTodoEvent {
                kind: TodoEventKind::Created,
                todo,
            },
            activity_opts(),
        )
        .await?;

        ctx.wait_condition(|s| s.deleted || s.todo.as_ref().is_some_and(|t| t.completed))
            .await?;

        let (deleted, todo) = ctx.state(|s| (s.deleted, s.todo.clone()));
        let todo = todo.expect("todo is set before the wait");
        let kind = if deleted {
            TodoEventKind::Deleted
        } else {
            TodoEventKind::Completed
        };
        ctx.execute_activity(
            TodoActivities::publish_event,
            PublishTodoEvent {
                kind,
                todo: todo.clone(),
            },
            activity_opts(),
        )
        .await?;

        Ok(if deleted { None } else { Some(todo) })
    }

    /// PATCH semantics, mirroring `Todo::apply_update` — reimplemented here
    /// because that helper stamps wall-clock `Utc::now()`, which is
    /// nondeterministic inside a workflow; workflow time is used instead.
    #[signal]
    pub fn update(&mut self, ctx: &mut SyncWorkflowContext<Self>, patch: UpdateTodo) {
        if self.deleted {
            return;
        }
        if let Some(todo) = self.todo.as_mut() {
            if let Some(title) = patch.title {
                todo.title = title;
            }
            if let Some(description) = patch.description {
                todo.description = description;
            }
            if let Some(completed) = patch.completed {
                todo.completed = completed;
            }
            if let Some(priority) = patch.priority {
                todo.priority = priority;
            }
            todo.updated_at = workflow_now(ctx.workflow_time());
        }
    }

    /// Marks the todo completed, letting `run` finish.
    #[signal]
    pub fn complete(&mut self, ctx: &mut SyncWorkflowContext<Self>) {
        if self.deleted {
            return;
        }
        if let Some(todo) = self.todo.as_mut() {
            todo.completed = true;
            todo.updated_at = workflow_now(ctx.workflow_time());
        }
    }

    /// Deletes the todo, letting `run` finish with `None`.
    #[signal]
    pub fn delete(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.deleted = true;
    }

    /// Read-model: the current snapshot (`None` before `run` initializes it).
    #[query]
    pub fn get_todo(&self, _ctx: &WorkflowContextView) -> Option<Todo> {
        self.todo.clone()
    }
}

/// Side effects live here, behind the same [`TodoEventPublisher`] abstraction
/// `todo_api` uses (NATS JetStream in production, no-op when NATS is down).
pub struct TodoActivities {
    publisher: Arc<dyn TodoEventPublisher>,
}

impl TodoActivities {
    pub fn new(publisher: Arc<dyn TodoEventPublisher>) -> Self {
        Self { publisher }
    }
}

#[activities]
impl TodoActivities {
    /// Publishes a lifecycle event to `todos.<kind>` — the stream the existing
    /// `todo_worker` consumes. Retried by Temporal on failure.
    #[activity]
    pub async fn publish_event(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: PublishTodoEvent,
    ) -> Result<(), ActivityError> {
        let event = match input.kind {
            TodoEventKind::Deleted => TodoEvent::deleted(input.todo.id),
            kind => TodoEvent::from_todo(kind, &input.todo),
        };
        tracing::info!(kind = ?input.kind, todo_id = %input.todo.id, "publishing todo event");
        self.publisher.publish(event).await?;
        Ok(())
    }
}

fn activity_opts() -> ActivityOptions {
    ActivityOptions::start_to_close_timeout(Duration::from_secs(10))
}

/// Workflow time (deterministic under replay) as `DateTime<Utc>`.
fn workflow_now(t: Option<SystemTime>) -> DateTime<Utc> {
    t.map(DateTime::<Utc>::from).unwrap_or_default()
}

/// Extracts the todo id from a `todo-<uuid>` workflow id.
fn todo_id(workflow_id: &str) -> Option<Uuid> {
    workflow_id
        .strip_prefix(WORKFLOW_ID_PREFIX)
        .and_then(|s| Uuid::parse_str(s).ok())
}
