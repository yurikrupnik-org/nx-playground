use async_trait::async_trait;
use database::BaseRepository;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use uuid::Uuid;

use crate::{
    entity,
    error::{TodoError, TodoResult},
    models::{CreateTodo, Todo, TodoFilter, UpdateTodo},
    repository::TodoRepository,
};

/// Postgres-backed [`TodoRepository`] built on the shared [`BaseRepository`].
pub struct PgTodoRepository {
    base: BaseRepository<entity::Entity>,
}

impl PgTodoRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            base: BaseRepository::new(db),
        }
    }
}

#[async_trait]
impl TodoRepository for PgTodoRepository {
    #[tracing::instrument(skip_all)]
    async fn create(&self, input: CreateTodo) -> TodoResult<Todo> {
        let active_model: entity::ActiveModel = input.into();
        let model = self
            .base
            .insert(active_model)
            .await
            .map_err(|e| TodoError::Database(e.to_string()))?;
        tracing::info!(todo_id = %model.id, "Created todo");
        Ok(model.into())
    }

    #[tracing::instrument(skip_all)]
    async fn get_by_id(&self, id: Uuid) -> TodoResult<Option<Todo>> {
        let model = self
            .base
            .find_by_id(id)
            .await
            .map_err(|e| TodoError::Database(e.to_string()))?;
        Ok(model.map(Into::into))
    }

    #[tracing::instrument(skip_all)]
    async fn list(&self, filter: TodoFilter) -> TodoResult<Vec<Todo>> {
        let mut query = entity::Entity::find();

        if let Some(completed) = filter.completed {
            query = query.filter(entity::Column::Completed.eq(completed));
        }
        if let Some(priority) = filter.priority {
            query = query.filter(entity::Column::Priority.eq(priority));
        }

        let models = query
            .order_by_desc(entity::Column::CreatedAt)
            .limit(filter.limit as u64)
            .offset(filter.offset as u64)
            .all(self.base.db())
            .await
            .map_err(|e| TodoError::Database(e.to_string()))?;

        Ok(models.into_iter().map(Into::into).collect())
    }

    #[tracing::instrument(skip_all)]
    async fn update(&self, id: Uuid, input: UpdateTodo) -> TodoResult<Todo> {
        let model = self
            .base
            .find_by_id(id)
            .await
            .map_err(|e| TodoError::Database(e.to_string()))?
            .ok_or(TodoError::NotFound(id))?;

        let mut todo: Todo = model.into();
        todo.apply_update(input);

        let active_model = entity::ActiveModel {
            id: Set(todo.id),
            title: Set(todo.title.clone()),
            description: Set(todo.description.clone()),
            completed: Set(todo.completed),
            priority: Set(todo.priority),
            created_at: Set(todo.created_at.into()),
            updated_at: Set(todo.updated_at.into()),
        };

        let updated = self
            .base
            .update(active_model)
            .await
            .map_err(|e| TodoError::Database(e.to_string()))?;
        tracing::info!(todo_id = %id, "Updated todo");
        Ok(updated.into())
    }

    #[tracing::instrument(skip_all)]
    async fn delete(&self, id: Uuid) -> TodoResult<bool> {
        let rows = self
            .base
            .delete_by_id(id)
            .await
            .map_err(|e| TodoError::Database(e.to_string()))?;
        Ok(rows > 0)
    }

    #[tracing::instrument(skip_all)]
    async fn count(&self) -> TodoResult<usize> {
        let count = entity::Entity::find()
            .count(self.base.db())
            .await
            .map_err(|e| TodoError::Database(e.to_string()))?;
        Ok(count as usize)
    }
}
