use std::sync::Arc;
use uuid::Uuid;
use validator::Validate;

use crate::{
    error::{CloudResourceError, CloudResourceResult},
    models::{CloudResource, CloudResourceFilter, CreateCloudResource, UpdateCloudResource},
    repository::CloudResourceRepository,
};

/// Cloud Resource Service - contains business logic and validation
#[derive(Clone)]
pub struct CloudResourceService<R: CloudResourceRepository> {
    repository: Arc<R>,
}

impl<R: CloudResourceRepository> CloudResourceService<R> {
    pub fn new(repository: R) -> Self {
        Self {
            repository: Arc::new(repository),
        }
    }

    /// Create a new cloud resource with validation
    pub async fn create(&self, input: CreateCloudResource) -> CloudResourceResult<CloudResource> {
        // Validate input
        input.validate()?;

        // Create resource
        let resource = self.repository.create(input).await?;

        tracing::info!(resource_id = %resource.id, "Created cloud resource");
        Ok(resource)
    }

    /// Get cloud resource by ID
    pub async fn get(&self, id: Uuid) -> CloudResourceResult<CloudResource> {
        let resource = self
            .repository
            .get_by_id(id)
            .await?
            .ok_or(CloudResourceError::NotFound(id))?;

        Ok(resource)
    }

    /// List cloud resources with filters
    pub async fn list(
        &self,
        filter: CloudResourceFilter,
    ) -> CloudResourceResult<Vec<CloudResource>> {
        self.repository.list(filter).await
    }

    /// List cloud resources by project ID
    pub async fn list_by_project(
        &self,
        project_id: Uuid,
    ) -> CloudResourceResult<Vec<CloudResource>> {
        self.repository.list_by_project(project_id).await
    }

    /// Update a cloud resource with validation
    pub async fn update(
        &self,
        id: Uuid,
        input: UpdateCloudResource,
    ) -> CloudResourceResult<CloudResource> {
        // Validate update
        input.validate()?;

        // Update resource
        let resource = self.repository.update(id, input).await?;

        tracing::info!(resource_id = %id, "Updated cloud resource");
        Ok(resource)
    }

    /// Delete a cloud resource (hard delete)
    pub async fn delete(&self, id: Uuid) -> CloudResourceResult<()> {
        let deleted = self.repository.delete(id).await?;
        if !deleted {
            return Err(CloudResourceError::NotFound(id));
        }
        tracing::info!(resource_id = %id, "Deleted cloud resource");
        Ok(())
    }

    /// Soft delete a cloud resource
    pub async fn soft_delete(&self, id: Uuid) -> CloudResourceResult<()> {
        self.repository.soft_delete(id).await?;
        tracing::info!(resource_id = %id, "Soft deleted cloud resource");
        Ok(())
    }

    /// Count cloud resources by project
    pub async fn count_by_project(&self, project_id: Uuid) -> CloudResourceResult<usize> {
        self.repository.count_by_project(project_id).await
    }
}
