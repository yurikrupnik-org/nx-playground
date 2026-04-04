use crate::models::{TaskPriority, TaskStatus};
use core_proc_macros::SeaOrmResource;
use sea_orm::entity::prelude::*;
use sea_orm::ActiveValue::Set;
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
            title: model.title,
            description: model.description,
            project_id: model.project_id,
            priority: model.priority,
            status: model.status,
            due_date: model.due_date.map(Into::into),
            created_at: model.created_at.into(),
            updated_at: model.updated_at.into(),
        }
    }
}

// Conversion from domain CreateTask to Sea-ORM ActiveModel
impl From<crate::models::CreateTask> for ActiveModel {
    fn from(input: crate::models::CreateTask) -> Self {
        ActiveModel {
            id: Set(Uuid::now_v7()),
            title: Set(input.title),
            description: Set(input.description),
            project_id: Set(input.project_id),
            priority: Set(input.priority),
            status: Set(input.status),
            due_date: Set(input.due_date.map(Into::into)),
            created_at: Set(chrono::Utc::now().into()),
            updated_at: Set(chrono::Utc::now().into()),
        }
    }
}
