use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    error::CloudResourceResult,
    models::{CloudResource, CloudResourceFilter, CreateCloudResource, UpdateCloudResource},
};

/// Repository trait for cloud resource operations
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait CloudResourceRepository: Send + Sync {
    /// Create a new cloud resource
    async fn create(&self, input: CreateCloudResource) -> CloudResourceResult<CloudResource>;

    /// Get cloud resource by ID
    async fn get_by_id(&self, id: Uuid) -> CloudResourceResult<Option<CloudResource>>;

    /// List cloud resources with optional filters
    async fn list(&self, filter: CloudResourceFilter) -> CloudResourceResult<Vec<CloudResource>>;

    /// List cloud resources by project ID
    async fn list_by_project(&self, project_id: Uuid) -> CloudResourceResult<Vec<CloudResource>>;

    /// Update a cloud resource
    async fn update(
        &self,
        id: Uuid,
        input: UpdateCloudResource,
    ) -> CloudResourceResult<CloudResource>;

    /// Delete a cloud resource (hard delete). Returns whether a row was removed.
    async fn delete(&self, id: Uuid) -> CloudResourceResult<bool>;

    /// Soft delete a cloud resource
    async fn soft_delete(&self, id: Uuid) -> CloudResourceResult<()>;

    /// Count cloud resources by project
    async fn count_by_project(&self, project_id: Uuid) -> CloudResourceResult<usize>;
}
