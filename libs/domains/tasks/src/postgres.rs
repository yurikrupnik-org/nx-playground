use async_trait::async_trait;
use chrono::Utc;
use database::BaseRepository;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use uuid::Uuid;

use crate::{
    entity,
    error::{TaskError, TaskResult},
    models::{CreateTask, Task, TaskFilter, TaskScope, UpdateTask},
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
    async fn create(&self, scope: TaskScope, input: CreateTask) -> TaskResult<Task> {
        let active_model: entity::ActiveModel = (scope, input).into();
        let model = self.base.insert(active_model).await?;
        tracing::info!(task_id = %model.id, org_ref = %model.org_ref, "Created task");
        Ok(model.into())
    }

    async fn get_by_id(&self, org_ref: &str, id: Uuid) -> TaskResult<Option<Task>> {
        let model = entity::Entity::find_by_id(id)
            .filter(entity::Column::OrgRef.eq(org_ref))
            .one(self.base.db())
            .await?;
        Ok(model.map(|m| m.into()))
    }

    async fn list(&self, scope: &TaskScope, filter: TaskFilter) -> TaskResult<Vec<Task>> {
        let mut query =
            entity::Entity::find().filter(entity::Column::OrgRef.eq(scope.org_ref.as_str()));

        // Apply filters. `mine` narrows to the caller within the already-enforced org;
        // the caller cannot name a different user.
        if filter.mine {
            query = query.filter(entity::Column::UserRef.eq(scope.user_ref.as_str()));
        }

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

    async fn update(&self, org_ref: &str, id: Uuid, input: UpdateTask) -> TaskResult<Task> {
        let model = entity::Entity::find_by_id(id)
            .filter(entity::Column::OrgRef.eq(org_ref))
            .one(self.base.db())
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

    async fn delete(&self, org_ref: &str, id: Uuid) -> TaskResult<bool> {
        let result = entity::Entity::delete_many()
            .filter(entity::Column::Id.eq(id))
            .filter(entity::Column::OrgRef.eq(org_ref))
            .exec(self.base.db())
            .await?;

        if result.rows_affected > 0 {
            tracing::info!(task_id = %id, "Deleted task");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn count(&self, org_ref: &str) -> TaskResult<usize> {
        let count = entity::Entity::find()
            .filter(entity::Column::OrgRef.eq(org_ref))
            .count(self.base.db())
            .await?;
        Ok(count as usize)
    }

    async fn count_by_project(&self, org_ref: &str, project_id: Uuid) -> TaskResult<usize> {
        let count = entity::Entity::find()
            .filter(entity::Column::OrgRef.eq(org_ref))
            .filter(entity::Column::ProjectId.eq(project_id))
            .count(self.base.db())
            .await?;
        Ok(count as usize)
    }

    async fn clear_project_refs(&self, project_id: Uuid) -> TaskResult<u64> {
        // One statement, not read-then-write: the rows to fix are exactly those
        // matching the filter, and a deleted project id is never reassigned, so
        // there is nothing to race with.
        //
        // `updated_at` moves with the write. The row's content really did
        // change, and callers/caches that key freshness on this column would
        // otherwise serve a stale `project_id` they believe is current.
        let result = entity::Entity::update_many()
            .col_expr(entity::Column::ProjectId, Expr::value(Option::<Uuid>::None))
            .col_expr(
                entity::Column::UpdatedAt,
                Expr::value(chrono::DateTime::<chrono::FixedOffset>::from(Utc::now())),
            )
            .filter(entity::Column::ProjectId.eq(project_id))
            .exec(self.base.db())
            .await?;

        if result.rows_affected > 0 {
            tracing::info!(
                %project_id,
                rows = result.rows_affected,
                "cleared task references to deleted project"
            );
        }
        Ok(result.rows_affected)
    }
}
