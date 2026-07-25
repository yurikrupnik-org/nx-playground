use async_trait::async_trait;
use chrono::Utc;
use database::BaseRepository;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, SqlErr,
};
use uuid::Uuid;

use crate::{
    entity,
    error::{ProjectError, ProjectResult},
    models::{CreateProject, Project, ProjectFilter, UpdateProject},
    repository::ProjectRepository,
};

pub struct PgProjectRepository {
    base: BaseRepository<entity::Entity>,
}

impl PgProjectRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            base: BaseRepository::new(db),
        }
    }
}

/// Map a write-path [`DbErr`] to a domain error.
///
/// A unique-constraint violation on `(user_id, name)` becomes
/// [`ProjectError::DuplicateName`] when a candidate name is known; anything
/// else keeps its `DbErr` source. This backstops the check-then-write races in
/// `create`/`update` and **requires a DB unique index on `(user_id, name)`** to
/// be authoritative.
fn map_write_err(err: sea_orm::DbErr, name: Option<&str>) -> ProjectError {
    match (err.sql_err(), name) {
        (Some(SqlErr::UniqueConstraintViolation(_)), Some(name)) => {
            ProjectError::DuplicateName(name.to_string())
        }
        _ => ProjectError::Database(err),
    }
}

#[async_trait]
impl ProjectRepository for PgProjectRepository {
    async fn create(&self, input: CreateProject) -> ProjectResult<Project> {
        // Fast-path duplicate-name check; the DB unique index (see
        // `map_write_err`) is the authoritative guard when this check races.
        if self.exists_by_name(input.user_id, &input.name).await? {
            return Err(ProjectError::DuplicateName(input.name));
        }

        let name = input.name.clone();
        let active_model: entity::ActiveModel = input.try_into()?;

        let model = self
            .base
            .insert(active_model)
            .await
            .map_err(|e| map_write_err(e, Some(&name)))?;

        tracing::info!(project_id = %model.id, "Created project");
        Ok(model.into())
    }

    async fn get_by_id(&self, id: Uuid) -> ProjectResult<Option<Project>> {
        let model = self.base.find_by_id(id).await?;
        Ok(model.map(|m| m.into()))
    }

    async fn list(&self, filter: ProjectFilter) -> ProjectResult<Vec<Project>> {
        let mut query = entity::Entity::find();

        // Apply filters
        if let Some(user_id) = filter.user_id {
            query = query.filter(entity::Column::UserId.eq(user_id));
        }

        if let Some(cloud_provider) = filter.cloud_provider {
            query = query.filter(entity::Column::CloudProvider.eq(cloud_provider));
        }

        if let Some(environment) = filter.environment {
            query = query.filter(entity::Column::Environment.eq(environment));
        }

        if let Some(status) = filter.status {
            query = query.filter(entity::Column::Status.eq(status));
        }

        if let Some(enabled) = filter.enabled {
            query = query.filter(entity::Column::Enabled.eq(enabled));
        }

        // Apply pagination and ordering
        query = query
            .order_by_desc(entity::Column::CreatedAt)
            .limit(filter.limit as u64)
            .offset(filter.offset as u64);

        let models = query.all(self.base.db()).await?;

        Ok(models.into_iter().map(|m| m.into()).collect())
    }

    async fn update(&self, id: Uuid, input: UpdateProject) -> ProjectResult<Project> {
        let model = self
            .base
            .find_by_id(id)
            .await?
            .ok_or(ProjectError::NotFound(id))?;

        // Fast-path duplicate-name check when the name is changing; the DB
        // unique index (see `map_write_err`) is the authoritative guard.
        if let Some(new_name) = &input.name {
            let name_taken = entity::Entity::find()
                .filter(entity::Column::UserId.eq(model.user_id))
                .filter(entity::Column::Name.eq(new_name))
                .filter(entity::Column::Id.ne(id))
                .one(self.base.db())
                .await?
                .is_some();

            if name_taken {
                return Err(ProjectError::DuplicateName(new_name.clone()));
            }
        }

        // Idiomatic sea-orm partial update: mutate only the fields present in
        // the DTO; untouched columns stay `Unchanged`, avoiding last-write-wins
        // clobbering of concurrent writers and per-field clones.
        let dup_name = input.name.clone();
        let mut active = model.into_active_model();
        if let Some(name) = input.name {
            active.name = Set(name);
        }
        if let Some(description) = input.description {
            active.description = Set(description);
        }
        if let Some(region) = input.region {
            active.region = Set(region);
        }
        if let Some(environment) = input.environment {
            active.environment = Set(environment);
        }
        if let Some(status) = input.status {
            active.status = Set(status);
        }
        if let Some(budget_limit) = input.budget_limit {
            active.budget_limit = Set(Some(budget_limit));
        }
        if let Some(tags) = input.tags {
            active.tags = Set(serde_json::to_value(&tags)
                .map_err(|e| ProjectError::Internal(format!("serialize tags: {e}")))?);
        }
        if let Some(enabled) = input.enabled {
            active.enabled = Set(enabled);
        }
        active.updated_at = Set(Utc::now().into());

        let updated_model = self
            .base
            .update(active)
            .await
            .map_err(|e| map_write_err(e, dup_name.as_deref()))?;

        tracing::info!(project_id = %id, "Updated project");
        Ok(updated_model.into())
    }

    async fn delete(&self, id: Uuid) -> ProjectResult<bool> {
        let rows_affected = self.base.delete_by_id(id).await?;

        if rows_affected > 0 {
            tracing::info!(project_id = %id, "Deleted project");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn exists_by_name(&self, user_id: Uuid, name: &str) -> ProjectResult<bool> {
        let exists = entity::Entity::find()
            .filter(entity::Column::UserId.eq(user_id))
            .filter(entity::Column::Name.eq(name))
            .one(self.base.db())
            .await?
            .is_some();

        Ok(exists)
    }

    async fn count_by_user(&self, user_id: Uuid) -> ProjectResult<usize> {
        let count = entity::Entity::find()
            .filter(entity::Column::UserId.eq(user_id))
            .count(self.base.db())
            .await?;

        Ok(count as usize)
    }
}
