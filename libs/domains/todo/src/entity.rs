//! SeaORM entity bridging the `todos` table to the domain `Todo` model.

use crate::models::TodoPriority;
use core_proc_macros::SeaOrmResource;
use sea_orm::entity::prelude::*;
use sea_orm::ActiveValue::Set;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, SeaOrmResource)]
#[sea_orm(table_name = "todos")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub title: String,
    #[sea_orm(column_type = "Text")]
    pub description: String,
    pub completed: bool,
    pub priority: TodoPriority,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for crate::models::Todo {
    fn from(model: Model) -> Self {
        Self {
            id: model.id,
            title: model.title,
            description: model.description,
            completed: model.completed,
            priority: model.priority,
            created_at: model.created_at.into(),
            updated_at: model.updated_at.into(),
        }
    }
}

impl From<crate::models::CreateTodo> for ActiveModel {
    fn from(input: crate::models::CreateTodo) -> Self {
        let now = chrono::Utc::now();
        ActiveModel {
            id: Set(Uuid::now_v7()),
            title: Set(input.title),
            description: Set(input.description),
            completed: Set(false),
            priority: Set(input.priority),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
    }
}
