use async_trait::async_trait;
use uuid::Uuid;

use crate::error::TaskResult;
use crate::models::{CreateTask, Task, TaskFilter, TaskScope, UpdateTask};

/// Repository trait for Task persistence
///
/// This trait defines the data access interface for tasks.
/// Implementations can use different storage backends (PostgreSQL, etc.)
///
/// Every method is tenant-scoped: `org_id` comes from the verified session
/// (never the client) and cross-tenant ids behave as not-found.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskRepository: Send + Sync {
    /// Create a new task owned by `scope`
    async fn create(&self, scope: TaskScope, input: CreateTask) -> TaskResult<Task>;

    /// Get a task by ID within the org
    async fn get_by_id(&self, org_ref: &str, id: Uuid) -> TaskResult<Option<Task>>;

    /// List the org's tasks with optional filters
    async fn list(&self, scope: &TaskScope, filter: TaskFilter) -> TaskResult<Vec<Task>>;

    /// Update an existing task within the org
    async fn update(&self, org_ref: &str, id: Uuid, input: UpdateTask) -> TaskResult<Task>;

    /// Delete a task by ID within the org
    async fn delete(&self, org_ref: &str, id: Uuid) -> TaskResult<bool>;

    /// Count the org's tasks
    async fn count(&self, org_ref: &str) -> TaskResult<usize>;

    /// Count the org's tasks for a project
    async fn count_by_project(&self, org_ref: &str, project_id: Uuid) -> TaskResult<usize>;

    /// Null the `project_id` of every task referencing `project_id`, returning
    /// how many rows changed.
    ///
    /// Deliberately **not** tenant-scoped, unlike every method above: the input
    /// is a `ProjectDeleted` fact from another service, which carries no org.
    /// Project ids are globally unique, so matching on the id alone is exact —
    /// and scoping it to one org would leave the other orgs' rows dangling.
    async fn clear_project_refs(&self, project_id: Uuid) -> TaskResult<u64>;
}
