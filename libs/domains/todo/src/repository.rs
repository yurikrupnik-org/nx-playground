use async_trait::async_trait;
use uuid::Uuid;

use crate::error::TodoResult;
use crate::models::{CreateTodo, Todo, TodoFilter, UpdateTodo};

/// Data-access seam for todos. Implementations back onto a storage engine.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TodoRepository: Send + Sync {
    async fn create(&self, input: CreateTodo) -> TodoResult<Todo>;
    async fn get_by_id(&self, id: Uuid) -> TodoResult<Option<Todo>>;
    async fn list(&self, filter: TodoFilter) -> TodoResult<Vec<Todo>>;
    async fn update(&self, id: Uuid, input: UpdateTodo) -> TodoResult<Todo>;
    async fn delete(&self, id: Uuid) -> TodoResult<bool>;
    async fn count(&self) -> TodoResult<usize>;
}
