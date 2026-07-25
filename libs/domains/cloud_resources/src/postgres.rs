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
    error::{CloudResourceError, CloudResourceResult},
    models::{
        CloudResource, CloudResourceFilter, CreateCloudResource, ResourceStatus,
        UpdateCloudResource,
    },
    repository::CloudResourceRepository,
};

pub struct PgCloudResourceRepository {
    base: BaseRepository<entity::Entity>,
}

impl PgCloudResourceRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            base: BaseRepository::new(db),
        }
    }
}

#[async_trait]
impl CloudResourceRepository for PgCloudResourceRepository {
    async fn create(&self, input: CreateCloudResource) -> CloudResourceResult<CloudResource> {
        let active_model: entity::ActiveModel = input.try_into()?;
        let model = self.base.insert(active_model).await?;
        model.try_into()
    }

    async fn get_by_id(&self, id: Uuid) -> CloudResourceResult<Option<CloudResource>> {
        let model = self.base.find_by_id(id).await?;
        model.map(TryInto::try_into).transpose()
    }

    async fn list(&self, filter: CloudResourceFilter) -> CloudResourceResult<Vec<CloudResource>> {
        let mut query = entity::Entity::find();

        // Apply filters
        if let Some(project_id) = filter.project_id {
            query = query.filter(entity::Column::ProjectId.eq(project_id));
        }

        if let Some(resource_type) = filter.resource_type {
            query = query.filter(entity::Column::ResourceType.eq(resource_type.to_string()));
        }

        if let Some(status) = filter.status {
            query = query.filter(entity::Column::Status.eq(status.to_string()));
        }

        if let Some(region) = filter.region {
            query = query.filter(entity::Column::Region.eq(region));
        }

        if let Some(enabled) = filter.enabled {
            query = query.filter(entity::Column::Enabled.eq(enabled));
        }

        // Apply pagination
        let models = query
            .order_by_desc(entity::Column::CreatedAt)
            .limit(filter.limit as u64)
            .offset(filter.offset as u64)
            .all(self.base.db())
            .await?;

        models.into_iter().map(TryInto::try_into).collect()
    }

    async fn list_by_project(&self, project_id: Uuid) -> CloudResourceResult<Vec<CloudResource>> {
        let models = entity::Entity::find()
            .filter(entity::Column::ProjectId.eq(project_id))
            .filter(entity::Column::DeletedAt.is_null())
            .order_by_desc(entity::Column::CreatedAt)
            .all(self.base.db())
            .await?;

        models.into_iter().map(TryInto::try_into).collect()
    }

    async fn update(
        &self,
        id: Uuid,
        input: UpdateCloudResource,
    ) -> CloudResourceResult<CloudResource> {
        let model = self
            .base
            .find_by_id(id)
            .await?
            .ok_or(CloudResourceError::NotFound(id))?;

        // Idiomatic sea-orm partial update: mutate only the fields present in
        // the DTO; untouched columns stay `Unchanged`, avoiding last-write-wins
        // clobbering of concurrent writers and per-field clones.
        let mut active = model.into_active_model();
        if let Some(name) = input.name {
            active.name = Set(name);
        }
        if let Some(status) = input.status {
            active.status = Set(status.to_string());
        }
        if let Some(region) = input.region {
            active.region = Set(region);
        }
        if let Some(configuration) = input.configuration {
            active.configuration = Set(configuration);
        }
        if let Some(cost_per_hour) = input.cost_per_hour {
            active.cost_per_hour = Set(Some(cost_per_hour));
            active.monthly_cost_estimate = Set(Some(cost_per_hour * 24.0 * 30.0));
        }
        if let Some(monthly_cost_estimate) = input.monthly_cost_estimate {
            active.monthly_cost_estimate = Set(Some(monthly_cost_estimate));
        }
        if let Some(tags) = input.tags {
            active.tags = Set(serde_json::to_value(&tags)
                .map_err(|e| CloudResourceError::Internal(format!("serialize tags: {e}")))?);
        }
        if let Some(enabled) = input.enabled {
            active.enabled = Set(enabled);
        }
        active.updated_at = Set(Utc::now().into());

        let updated_model = self.base.update(active).await?;
        updated_model.try_into()
    }

    async fn delete(&self, id: Uuid) -> CloudResourceResult<bool> {
        let rows_affected = self.base.delete_by_id(id).await?;
        Ok(rows_affected > 0)
    }

    async fn soft_delete(&self, id: Uuid) -> CloudResourceResult<()> {
        let model = self
            .base
            .find_by_id(id)
            .await?
            .ok_or(CloudResourceError::NotFound(id))?;

        // Partial update: flip status + timestamps only.
        let now = Utc::now();
        let mut active = model.into_active_model();
        active.status = Set(ResourceStatus::Deleted.to_string());
        active.deleted_at = Set(Some(now.into()));
        active.updated_at = Set(now.into());

        self.base.update(active).await?;
        Ok(())
    }

    async fn count_by_project(&self, project_id: Uuid) -> CloudResourceResult<usize> {
        let count = entity::Entity::find()
            .filter(entity::Column::ProjectId.eq(project_id))
            .filter(entity::Column::DeletedAt.is_null())
            .count(self.base.db())
            .await?;

        Ok(count as usize)
    }
}
