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
        // Convert CreateTask to ActiveModel
        let active_model: entity::ActiveModel = input.into();

        // Insert using base repository
        let model = self
            .base
            .insert(active_model)
            .await
            .map_err(|e| TaskError::Internal(format!("Database error: {}", e)))?;

        tracing::info!(task_id = %model.id, "Created task");
        Ok(model.into())
    }

    async fn get_by_id(&self, id: Uuid) -> TaskResult<Option<Task>> {
        let model = self
            .base
            .find_by_id(id)
            .await
            .map_err(|e| TaskError::Internal(format!("Database error: {}", e)))?;

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

        // Apply pagination and ordering
        query = query
            .order_by_desc(entity::Column::CreatedAt)
            .limit(filter.limit as u64)
            .offset(filter.offset as u64);

        let models = query
            .all(self.base.db())
            .await
            .map_err(|e| TaskError::Internal(format!("Database error: {}", e)))?;

        Ok(models.into_iter().map(|m| m.into()).collect())
    }

    async fn update(&self, id: Uuid, input: UpdateTask) -> TaskResult<Task> {
        // Fetch existing task
        let model = self
            .base
            .find_by_id(id)
            .await
            .map_err(|e| TaskError::Internal(format!("Database error: {}", e)))?
            .ok_or(TaskError::NotFound(id))?;

        // Convert to domain model
        let mut task: Task = model.into();

        // Apply updates
        task.apply_update(input);

        // Convert back to ActiveModel for update
        let active_model: entity::ActiveModel = entity::ActiveModel {
            id: Set(task.id),
            title: Set(task.title.clone()),
            description: Set(task.description.clone()),
            project_id: Set(task.project_id),
            priority: Set(task.priority),
            status: Set(task.status),
            due_date: Set(task.due_date.map(Into::into)),
            created_at: Set(task.created_at.into()),
            updated_at: Set(task.updated_at.into()),
        };

        // Update using base repository
        let updated_model = self
            .base
            .update(active_model)
            .await
            .map_err(|e| TaskError::Internal(format!("Database error: {}", e)))?;

        tracing::info!(task_id = %id, "Updated task");
        Ok(updated_model.into())
    }

    async fn delete(&self, id: Uuid) -> TaskResult<bool> {
        let rows_affected = self
            .base
            .delete_by_id(id)
            .await
            .map_err(|e| TaskError::Internal(format!("Database error: {}", e)))?;

        if rows_affected > 0 {
            tracing::info!(task_id = %id, "Deleted task");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn count(&self) -> TaskResult<usize> {
        let count = entity::Entity::find()
            .count(self.base.db())
            .await
            .map_err(|e| TaskError::Internal(format!("Database error: {}", e)))?;

        Ok(count as usize)
    }

    async fn count_by_project(&self, project_id: Uuid) -> TaskResult<usize> {
        let count = entity::Entity::find()
            .filter(entity::Column::ProjectId.eq(project_id))
            .count(self.base.db())
            .await
            .map_err(|e| TaskError::Internal(format!("Database error: {}", e)))?;

        Ok(count as usize)
    }
}
