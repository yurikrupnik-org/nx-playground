//! Business logic for todos. Validates input, delegates persistence to a
//! [`TodoRepository`], and publishes lifecycle [`TodoEvent`]s best-effort
//! (a publish failure is logged, never surfaced to the caller — observability
//! must not take the write path down).

use std::sync::Arc;

use tracing::{instrument, warn};
use uuid::Uuid;
use validator::Validate;

use crate::error::{TodoError, TodoResult};
use crate::events::{TodoEvent, TodoEventKind, TodoEventPublisher};
use crate::models::{CreateTodo, Todo, TodoFilter, UpdateTodo};
use crate::repository::TodoRepository;

/// Service layer for the Todo domain.
///
/// Cheap to clone (two `Arc`s), and cloneable for ANY repository: a derived
/// `Clone` would demand `R: Clone`, which the Postgres-backed repositories do
/// not offer and the transports (REST router + gRPC service) do not need.
pub struct TodoService<R: TodoRepository> {
    repository: Arc<R>,
    publisher: Arc<dyn TodoEventPublisher>,
}

impl<R: TodoRepository> Clone for TodoService<R> {
    fn clone(&self) -> Self {
        Self {
            repository: Arc::clone(&self.repository),
            publisher: Arc::clone(&self.publisher),
        }
    }
}

impl<R: TodoRepository> TodoService<R> {
    pub fn new(repository: R, publisher: Arc<dyn TodoEventPublisher>) -> Self {
        Self {
            repository: Arc::new(repository),
            publisher,
        }
    }

    async fn emit(&self, event: TodoEvent) {
        if let Err(e) = self.publisher.publish(event).await {
            warn!(error = %e, "failed to publish todo event");
        }
    }

    #[instrument(skip(self, input), fields(title = %input.title))]
    pub async fn create_todo(&self, input: CreateTodo) -> TodoResult<Todo> {
        input.validate()?;
        let todo = self.repository.create(input).await?;
        self.emit(TodoEvent::from_todo(TodoEventKind::Created, &todo))
            .await;
        Ok(todo)
    }

    #[instrument(skip(self), fields(todo_id = %id))]
    pub async fn get_todo(&self, id: Uuid) -> TodoResult<Todo> {
        self.repository
            .get_by_id(id)
            .await?
            .ok_or(TodoError::NotFound(id))
    }

    pub async fn list_todos(&self, filter: TodoFilter) -> TodoResult<Vec<Todo>> {
        self.repository.list(filter).await
    }

    #[instrument(skip(self, input), fields(todo_id = %id))]
    pub async fn update_todo(&self, id: Uuid, input: UpdateTodo) -> TodoResult<Todo> {
        input.validate()?;
        let completed_change = input.completed;
        let todo = self.repository.update(id, input).await?;

        // Distinguish completion transitions for a richer event stream.
        let kind = match completed_change {
            Some(true) => TodoEventKind::Completed,
            Some(false) => TodoEventKind::Uncompleted,
            None => TodoEventKind::Updated,
        };
        self.emit(TodoEvent::from_todo(kind, &todo)).await;
        Ok(todo)
    }

    #[instrument(skip(self), fields(todo_id = %id))]
    pub async fn delete_todo(&self, id: Uuid) -> TodoResult<()> {
        let deleted = self.repository.delete(id).await?;
        if !deleted {
            return Err(TodoError::NotFound(id));
        }
        self.emit(TodoEvent::deleted(id)).await;
        Ok(())
    }

    #[instrument(skip(self), fields(todo_id = %id))]
    pub async fn complete_todo(&self, id: Uuid) -> TodoResult<Todo> {
        let todo = self
            .repository
            .update(
                id,
                UpdateTodo {
                    completed: Some(true),
                    ..Default::default()
                },
            )
            .await?;
        self.emit(TodoEvent::from_todo(TodoEventKind::Completed, &todo))
            .await;
        Ok(todo)
    }

    #[instrument(skip(self), fields(todo_id = %id))]
    pub async fn uncomplete_todo(&self, id: Uuid) -> TodoResult<Todo> {
        let todo = self
            .repository
            .update(
                id,
                UpdateTodo {
                    completed: Some(false),
                    ..Default::default()
                },
            )
            .await?;
        self.emit(TodoEvent::from_todo(TodoEventKind::Uncompleted, &todo))
            .await;
        Ok(todo)
    }

    pub async fn count_todos(&self) -> TodoResult<usize> {
        self.repository.count().await
    }
}
