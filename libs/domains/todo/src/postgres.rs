use async_trait::async_trait;
use chrono::Utc;
use database::BaseRepository;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
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
        let model = self.base.insert(active_model).await?;
        tracing::info!(todo_id = %model.id, "Created todo");
        Ok(model.into())
    }

    #[tracing::instrument(skip_all)]
    async fn get_by_id(&self, id: Uuid) -> TodoResult<Option<Todo>> {
        let model = self.base.find_by_id(id).await?;
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
            .await?;

        Ok(models.into_iter().map(Into::into).collect())
    }

    #[tracing::instrument(skip_all)]
    async fn update(&self, id: Uuid, input: UpdateTodo) -> TodoResult<Todo> {
        let model = self
            .base
            .find_by_id(id)
            .await?
            .ok_or(TodoError::NotFound(id))?;

        // Idiomatic sea-orm partial update: mutate only the fields present in the
        // DTO; untouched columns stay `Unchanged` so concurrent writers are not
        // clobbered and per-field String clones are avoided.
        let mut active = model.into_active_model();
        if let Some(title) = input.title {
            active.title = Set(title);
        }
        if let Some(description) = input.description {
            active.description = Set(description);
        }
        if let Some(completed) = input.completed {
            active.completed = Set(completed);
        }
        if let Some(priority) = input.priority {
            active.priority = Set(priority);
        }
        active.updated_at = Set(Utc::now().into());

        let updated = self.base.update(active).await?;
        tracing::info!(todo_id = %id, "Updated todo");
        Ok(updated.into())
    }

    #[tracing::instrument(skip_all)]
    async fn delete(&self, id: Uuid) -> TodoResult<bool> {
        let rows = self.base.delete_by_id(id).await?;
        Ok(rows > 0)
    }

    #[tracing::instrument(skip_all)]
    async fn count(&self) -> TodoResult<usize> {
        let count = entity::Entity::find().count(self.base.db()).await?;
        Ok(count as usize)
    }
}
