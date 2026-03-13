use std::sync::Arc;
use tracing::instrument;
use uuid::Uuid;
use validator::Validate;

use crate::error::{TaskError, TaskResult};
use crate::models::{CreateTask, Task, TaskFilter, TaskStatus, UpdateTask};
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

    /// Create a new task with validation
    #[instrument(skip(self, input), fields(task_title = %input.title))]
    pub async fn create_task(&self, input: CreateTask) -> TaskResult<Task> {
        // Validate input
        input
            .validate()
            .map_err(|e| TaskError::Validation(e.to_string()))?;

        self.repository.create(input).await
    }

    /// Get a task by ID
    #[instrument(skip(self), fields(task_id = %id))]
    pub async fn get_task(&self, id: Uuid) -> TaskResult<Task> {
        self.repository
            .get_by_id(id)
            .await?
            .ok_or(TaskError::NotFound(id))
    }

    /// List tasks with filters
    pub async fn list_tasks(&self, filter: TaskFilter) -> TaskResult<Vec<Task>> {
        self.repository.list(filter).await
    }

    /// Update a task
    #[instrument(skip(self, input), fields(task_id = %id))]
    pub async fn update_task(&self, id: Uuid, input: UpdateTask) -> TaskResult<Task> {
        // Validate input
        input
            .validate()
            .map_err(|e| TaskError::Validation(e.to_string()))?;

        self.repository.update(id, input).await
    }

    /// Delete a task
    #[instrument(skip(self), fields(task_id = %id))]
    pub async fn delete_task(&self, id: Uuid) -> TaskResult<()> {
        let deleted = self.repository.delete(id).await?;

        if !deleted {
            return Err(TaskError::NotFound(id));
        }

        Ok(())
    }

    /// Mark a task as completed
    pub async fn complete_task(&self, id: Uuid) -> TaskResult<Task> {
        self.repository
            .update(
                id,
                UpdateTask {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
    }

    /// Mark a task as incomplete
    pub async fn uncomplete_task(&self, id: Uuid) -> TaskResult<Task> {
        self.repository
            .update(
                id,
                UpdateTask {
                    status: Some(TaskStatus::Todo),
                    ..Default::default()
                },
            )
            .await
    }

    /// Count all tasks
    pub async fn count_tasks(&self) -> TaskResult<usize> {
        self.repository.count().await
    }

    /// Count tasks for a project
    pub async fn count_tasks_by_project(&self, project_id: Uuid) -> TaskResult<usize> {
        self.repository.count_by_project(project_id).await
    }
}
