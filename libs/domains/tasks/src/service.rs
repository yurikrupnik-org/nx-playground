use std::sync::Arc;
use tracing::instrument;
use uuid::Uuid;
use validator::Validate;

use crate::error::{TaskError, TaskResult};
use crate::models::{CreateTask, Task, TaskFilter, TaskScope, TaskStatus, UpdateTask};
use crate::repository::TaskRepository;

/// Service layer for Task business logic
pub struct TaskService<R: TaskRepository> {
    repository: Arc<R>,
}

/// Hand-written so cloning only bumps the `Arc`; `derive(Clone)` would demand
/// `R: Clone`, which a repository holding a DB handle has no reason to be —
/// and the server needs a second handle to hand to the project-events
/// consumer.
impl<R: TaskRepository> Clone for TaskService<R> {
    fn clone(&self) -> Self {
        Self {
            repository: Arc::clone(&self.repository),
        }
    }
}

impl<R: TaskRepository> TaskService<R> {
    pub fn new(repository: R) -> Self {
        Self {
            repository: Arc::new(repository),
        }
    }

    /// Create a new task with validation, owned by `scope`
    #[instrument(skip(self, input), fields(task_title = %input.title, org_ref = %scope.org_ref))]
    pub async fn create_task(&self, scope: TaskScope, input: CreateTask) -> TaskResult<Task> {
        // Validate input
        input
            .validate()
            .map_err(|e| TaskError::Validation(e.to_string()))?;

        self.repository.create(scope, input).await
    }

    /// Get a task by ID within the org
    #[instrument(skip(self), fields(task_id = %id))]
    pub async fn get_task(&self, org_ref: &str, id: Uuid) -> TaskResult<Task> {
        self.repository
            .get_by_id(org_ref, id)
            .await?
            .ok_or(TaskError::NotFound(id))
    }

    /// List the org's tasks with filters
    pub async fn list_tasks(&self, scope: &TaskScope, filter: TaskFilter) -> TaskResult<Vec<Task>> {
        self.repository.list(scope, filter).await
    }

    /// Update a task within the org
    #[instrument(skip(self, input), fields(task_id = %id))]
    pub async fn update_task(
        &self,
        org_ref: &str,
        id: Uuid,
        input: UpdateTask,
    ) -> TaskResult<Task> {
        // Validate input
        input
            .validate()
            .map_err(|e| TaskError::Validation(e.to_string()))?;

        self.repository.update(org_ref, id, input).await
    }

    /// Delete a task within the org
    #[instrument(skip(self), fields(task_id = %id))]
    pub async fn delete_task(&self, org_ref: &str, id: Uuid) -> TaskResult<()> {
        let deleted = self.repository.delete(org_ref, id).await?;

        if !deleted {
            return Err(TaskError::NotFound(id));
        }

        Ok(())
    }

    /// Mark a task as completed
    pub async fn complete_task(&self, org_ref: &str, id: Uuid) -> TaskResult<Task> {
        self.repository
            .update(
                org_ref,
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
    pub async fn uncomplete_task(&self, org_ref: &str, id: Uuid) -> TaskResult<Task> {
        self.repository
            .update(
                org_ref,
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
    pub async fn count_tasks(&self, org_ref: &str) -> TaskResult<usize> {
        self.repository.count(org_ref).await
    }

    /// Count the org's tasks for a project
    pub async fn count_tasks_by_project(
        &self,
        org_ref: &str,
        project_id: Uuid,
    ) -> TaskResult<usize> {
        self.repository.count_by_project(org_ref, project_id).await
    }

    /// Drop every reference to a project that no longer exists, returning how
    /// many tasks were changed.
    ///
    /// Driven by the `ProjectDeleted` fact from the service that owns projects,
    /// so it is cross-tenant by nature — see
    /// [`TaskRepository::clear_project_refs`].
    #[instrument(skip(self), fields(project_id = %project_id))]
    pub async fn clear_project_refs(&self, project_id: Uuid) -> TaskResult<u64> {
        self.repository.clear_project_refs(project_id).await
    }
}
