use std::sync::Arc;
use tracing::instrument;
use uuid::Uuid;
use validator::Validate;

use crate::error::{TaskError, TaskResult};
use crate::models::{CreateTask, Task, TaskFilter, TaskScope, TaskStatus, UpdateTask};
use crate::repository::TaskRepository;

/// Service layer for Task business logic
#[derive(Clone)]
pub struct TaskService<R: TaskRepository> {
    repository: Arc<R>,
}

impl<R: TaskRepository> TaskService<R> {
    pub fn new(repository: R) -> Self {
        Self {
            repository: Arc::new(repository),
        }
    }

    /// Create a new task with validation, owned by `scope`
    #[instrument(skip(self, input), fields(task_title = %input.title, org_id = %scope.org_id))]
    pub async fn create_task(&self, scope: TaskScope, input: CreateTask) -> TaskResult<Task> {
        // Validate input
        input
            .validate()
            .map_err(|e| TaskError::Validation(e.to_string()))?;

        self.repository.create(scope, input).await
    }

    /// Get a task by ID within the org
    #[instrument(skip(self), fields(task_id = %id))]
    pub async fn get_task(&self, org_id: Uuid, id: Uuid) -> TaskResult<Task> {
        self.repository
            .get_by_id(org_id, id)
            .await?
            .ok_or(TaskError::NotFound(id))
    }

    /// List the org's tasks with filters
    pub async fn list_tasks(&self, org_id: Uuid, filter: TaskFilter) -> TaskResult<Vec<Task>> {
        self.repository.list(org_id, filter).await
    }

    /// Update a task within the org
    #[instrument(skip(self, input), fields(task_id = %id))]
    pub async fn update_task(&self, org_id: Uuid, id: Uuid, input: UpdateTask) -> TaskResult<Task> {
        // Validate input
        input
            .validate()
            .map_err(|e| TaskError::Validation(e.to_string()))?;

        self.repository.update(org_id, id, input).await
    }

    /// Delete a task within the org
    #[instrument(skip(self), fields(task_id = %id))]
    pub async fn delete_task(&self, org_id: Uuid, id: Uuid) -> TaskResult<()> {
        let deleted = self.repository.delete(org_id, id).await?;

        if !deleted {
            return Err(TaskError::NotFound(id));
        }

        Ok(())
    }

    /// Mark a task as completed
    pub async fn complete_task(&self, org_id: Uuid, id: Uuid) -> TaskResult<Task> {
        self.repository
            .update(
                org_id,
                id,
                UpdateTask {
                    completed: Some(true),
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
    }

    /// Mark a task as incomplete
    pub async fn uncomplete_task(&self, org_id: Uuid, id: Uuid) -> TaskResult<Task> {
        self.repository
            .update(
                org_id,
                id,
                UpdateTask {
                    completed: Some(false),
                    status: Some(TaskStatus::Todo),
                    ..Default::default()
                },
            )
            .await
    }

    /// Count the org's tasks
    pub async fn count_tasks(&self, org_id: Uuid) -> TaskResult<usize> {
        self.repository.count(org_id).await
    }

    /// Count the org's tasks for a project
    pub async fn count_tasks_by_project(&self, org_id: Uuid, project_id: Uuid) -> TaskResult<usize> {
        self.repository.count_by_project(org_id, project_id).await
    }
}
