//! Todo lifecycle on Restate — the virtual-object counterpart to
//! `todo_temporal` (workflow-as-entity) and `todo_api` + `todo_worker`
//! (Postgres row + JetStream events).
//!
//! One [`TodoObject`] keyed by a todo UUID *is* one todo: the snapshot lives in
//! Restate's per-key state (no Postgres row), exclusive handlers are serialized
//! per key so concurrent edits to one todo never race, and the shared `get`
//! handler reads without taking that lock. Side effects — [`TodoEvent`]s on the
//! `todos.>` JetStream subjects `todo_worker` consumes — run inside `ctx.run`,
//! whose results are journaled: a crash after a publish resumes past it rather
//! than re-running the handler. The publish itself is at-least-once (a crash
//! between NATS acking and Restate journaling the result repeats it).
//!
//! Restate's ingress is plain HTTP, so unlike `todo_temporal` there is no
//! starter binary. With the compose server (ingress published on host port
//! 18080, since 8080 is todo-api's) and this endpoint registered (see
//! `main.rs`), `<id>` being any UUID:
//!
//! ```text
//! curl localhost:18080/TodoObject/<id>/create --json '{"title":"try restate"}'
//! curl localhost:18080/TodoObject/<id>/update --json '{"priority":"high"}'
//! curl -X POST localhost:18080/TodoObject/<id>/get
//! curl -X POST localhost:18080/TodoObject/<id>/complete
//! curl -X POST localhost:18080/TodoObject/<id>/delete
//! ```

use std::sync::Arc;

use chrono::{DateTime, Utc};
use domain_todo::{CreateTodo, Todo, TodoEvent, TodoEventKind, TodoEventPublisher, UpdateTodo};
use restate_sdk::prelude::*;
use uuid::Uuid;
use validator::Validate;

/// State key holding the todo snapshot inside each object.
const TODO: &str = "todo";

/// One virtual object per todo, addressed by the todo id.
pub struct TodoObject {
    publisher: Arc<dyn TodoEventPublisher>,
}

impl TodoObject {
    pub fn new(publisher: Arc<dyn TodoEventPublisher>) -> Self {
        Self { publisher }
    }
}

#[object]
impl TodoObject {
    /// Creates the todo the object key names. 409 if it already exists, 400 on
    /// a key that is not a UUID or an input that fails validation.
    #[handler]
    async fn create(
        &self,
        ctx: ObjectContext<'_>,
        input: Json<CreateTodo>,
    ) -> HandlerResult<Json<Todo>> {
        let id = Uuid::parse_str(ctx.key()).map_err(|e| {
            TerminalError::new_with_code(400, format!("object key must be a todo UUID: {e}"))
        })?;
        let input = input.into_inner();
        input.validate().map_err(bad_request)?;
        if ctx.get::<Json<Todo>>(TODO).await?.is_some() {
            return Err(
                TerminalError::new_with_code(409, format!("todo {id} already exists")).into(),
            );
        }

        let now = journaled_now(&ctx).await?;
        let todo = Todo {
            id,
            title: input.title,
            description: input.description,
            completed: false,
            priority: input.priority,
            created_at: now,
            updated_at: now,
        };
        ctx.set(TODO, Json(todo.clone()));
        self.publish(&ctx, TodoEventKind::Created, &todo).await?;
        Ok(Json(todo))
    }

    /// Current snapshot; 404 before `create` and after `delete`.
    #[handler]
    async fn get(&self, ctx: SharedObjectContext<'_>) -> HandlerResult<Json<Todo>> {
        existing(ctx.key(), ctx.get(TODO).await?).map(Json)
    }

    /// PATCH semantics (`Todo::apply_update`); the event kind follows the
    /// `completed` field exactly as `todo_api` does.
    #[handler]
    async fn update(
        &self,
        ctx: ObjectContext<'_>,
        patch: Json<UpdateTodo>,
    ) -> HandlerResult<Json<Todo>> {
        let patch = patch.into_inner();
        patch.validate().map_err(bad_request)?;
        self.apply(&ctx, patch).await.map(Json)
    }

    /// Shorthand for `update` with `completed: true`.
    #[handler]
    async fn complete(&self, ctx: ObjectContext<'_>) -> HandlerResult<Json<Todo>> {
        let patch = UpdateTodo {
            completed: Some(true),
            ..Default::default()
        };
        self.apply(&ctx, patch).await.map(Json)
    }

    /// Clears the object's state, so the key can be created again. 404 if absent.
    #[handler]
    async fn delete(&self, ctx: ObjectContext<'_>) -> HandlerResult<()> {
        let todo = existing(ctx.key(), ctx.get(TODO).await?)?;
        ctx.clear(TODO);
        let publisher = Arc::clone(&self.publisher);
        ctx.run(|| async move {
            publisher.publish(TodoEvent::deleted(todo.id)).await?;
            Ok(())
        })
        .name("publish todos.deleted")
        .await?;
        Ok(())
    }
}

impl TodoObject {
    async fn apply(&self, ctx: &ObjectContext<'_>, patch: UpdateTodo) -> HandlerResult<Todo> {
        let mut todo = existing(ctx.key(), ctx.get(TODO).await?)?;
        let kind = TodoEventKind::for_update(patch.completed);
        let now = journaled_now(ctx).await?;
        todo.apply_update(patch);
        // `apply_update` stamps the wall clock, which would differ on replay;
        // the journaled timestamp is the one Restate will hand back every time.
        todo.updated_at = now;
        ctx.set(TODO, Json(todo.clone()));
        self.publish(ctx, kind, &todo).await?;
        Ok(todo)
    }

    /// Publishes inside `ctx.run`: retried with backoff until NATS accepts it,
    /// then journaled, so a replay never publishes a second time.
    async fn publish(
        &self,
        ctx: &ObjectContext<'_>,
        kind: TodoEventKind,
        todo: &Todo,
    ) -> HandlerResult<()> {
        let publisher = Arc::clone(&self.publisher);
        let todo = todo.clone();
        ctx.run(|| async move {
            publisher.publish(TodoEvent::from_todo(kind, &todo)).await?;
            Ok(())
        })
        .name(format!("publish todos.{}", kind.subject_suffix()))
        .await?;
        Ok(())
    }
}

/// Wall clock read once and journaled, so replays see the same instant.
async fn journaled_now(ctx: &ObjectContext<'_>) -> HandlerResult<DateTime<Utc>> {
    let now = ctx
        .run(|| async { Ok(Json(Utc::now())) })
        .name("now")
        .await?;
    Ok(now.into_inner())
}

fn existing(key: &str, state: Option<Json<Todo>>) -> HandlerResult<Todo> {
    state
        .map(Json::into_inner)
        .ok_or_else(|| TerminalError::new_with_code(404, format!("todo {key} not found")).into())
}

fn bad_request(e: impl std::fmt::Display) -> TerminalError {
    TerminalError::new_with_code(400, e.to_string())
}
