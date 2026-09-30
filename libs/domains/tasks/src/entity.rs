use crate::models::{TaskPriority, TaskStatus};
use core_proc_macros::SeaOrmResource;
use sea_orm::ActiveValue::Set;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Sea-ORM Entity for Tasks table
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, SeaOrmResource)]
#[sea_orm(table_name = "tasks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub title: String,
    #[sea_orm(column_type = "Text")]
    pub description: String,
    pub completed: bool,
    #[sea_orm(column_type = "Text")]
    pub org_ref: String,
    #[sea_orm(column_type = "Text")]
    pub user_ref: String,
    pub project_id: Option<Uuid>,
    pub priority: TaskPriority,
    pub status: TaskStatus,
    pub due_date: Option<DateTimeWithTimeZone>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

// Conversion from Sea-ORM Model to domain Task
impl From<Model> for crate::models::Task {
    fn from(model: Model) -> Self {
        Self {
            id: model.id,
            org_ref: model.org_ref,
            user_ref: model.user_ref,
            title: model.title,
            description: model.description,
            completed: model.completed,
            project_id: model.project_id,
            priority: model.priority,
            status: model.status,
            due_date: model.due_date.map(Into::into),
            created_at: model.created_at.into(),
            updated_at: model.updated_at.into(),
        }
    }
}

// Conversion from tenant scope + domain CreateTask to Sea-ORM ActiveModel.
// The scope is derived by this service from the caller's verified access token,
// never from the client payload.
impl From<(crate::models::TaskScope, crate::models::CreateTask)> for ActiveModel {
    fn from((scope, input): (crate::models::TaskScope, crate::models::CreateTask)) -> Self {
        let now = chrono::Utc::now();
        ActiveModel {
            id: Set(Uuid::now_v7()),
            org_ref: Set(scope.org_ref),
            user_ref: Set(scope.user_ref),
            title: Set(input.title),
            description: Set(input.description),
            completed: Set(false),
            project_id: Set(input.project_id),
            priority: Set(input.priority),
            status: Set(input.status),
            due_date: Set(input.due_date.map(Into::into)),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
    }
}
