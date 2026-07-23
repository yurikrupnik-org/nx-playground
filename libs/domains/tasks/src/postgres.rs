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
    error::{TaskError, TaskResult},
    models::{CreateTask, Task, TaskFilter, UpdateTask},
    repository::TaskRepository,
};

pub struct PgTaskRepository {
    base: BaseRepository<entity::Entity>,
}

impl PgTaskRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            base: BaseRepository::new(db),
        }
    }
}

#[async_trait]
impl TaskRepository for PgTaskRepository {
    async fn create(&self, input: CreateTask) -> TaskResult<Task> {
        let active_model: entity::ActiveModel = input.into();
        let model = self.base.insert(active_model).await?;
        tracing::info!(task_id = %model.id, "Created task");
        Ok(model.into())
    }

    async fn get_by_id(&self, id: Uuid) -> TaskResult<Option<Task>> {
        let model = self.base.find_by_id(id).await?;
        Ok(model.map(|m| m.into()))
    }

    async fn list(&self, filter: TaskFilter) -> TaskResult<Vec<Task>> {
        let mut query = entity::Entity::find();

        // Apply filters
        if let Some(project_id) = filter.project_id {
            query = query.filter(entity::Column::ProjectId.eq(project_id));
        }

        if let Some(status) = filter.status {
            query = query.filter(entity::Column::Status.eq(status));
        }

        if let Some(priority) = filter.priority {
            query = query.filter(entity::Column::Priority.eq(priority));
        }

        if let Some(completed) = filter.completed {
            query = query.filter(entity::Column::Completed.eq(completed));
        }

        // Apply pagination and ordering
        query = query
            .order_by_desc(entity::Column::CreatedAt)
            .limit(filter.limit as u64)
            .offset(filter.offset as u64);

        let models = query.all(self.base.db()).await?;

        Ok(models.into_iter().map(|m| m.into()).collect())
    }

    async fn update(&self, id: Uuid, input: UpdateTask) -> TaskResult<Task> {
        let model = self
            .base
            .find_by_id(id)
            .await?
            .ok_or(TaskError::NotFound(id))?;

        // Idiomatic sea-orm partial update: mutate only the fields present in the
        // DTO; untouched columns stay `Unchanged`, avoiding last-write-wins
        // clobbering of concurrent writers and per-field String clones.
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
        if let Some(project_id) = input.project_id {
            active.project_id = Set(project_id);
        }
        if let Some(priority) = input.priority {
            active.priority = Set(priority);
        }
        if let Some(status) = input.status {
            active.status = Set(status);
        }
        if let Some(due_date) = input.due_date {
            active.due_date = Set(due_date.map(Into::into));
        }
        active.updated_at = Set(Utc::now().into());

        let updated_model = self.base.update(active).await?;
        tracing::info!(task_id = %id, "Updated task");
        Ok(updated_model.into())
    }

    async fn delete(&self, id: Uuid) -> TaskResult<bool> {
        let rows_affected = self.base.delete_by_id(id).await?;

        if rows_affected > 0 {
            tracing::info!(task_id = %id, "Deleted task");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn count(&self) -> TaskResult<usize> {
        let count = entity::Entity::find().count(self.base.db()).await?;
        Ok(count as usize)
    }

    async fn count_by_project(&self, project_id: Uuid) -> TaskResult<usize> {
        let count = entity::Entity::find()
            .filter(entity::Column::ProjectId.eq(project_id))
            .count(self.base.db())
            .await?;
        Ok(count as usize)
    }
}
