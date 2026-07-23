use crate::models::{ResourceStatus, ResourceType, Tag};
use core_proc_macros::SeaOrmResource;
use sea_orm::ActiveValue::Set;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Sea-ORM Entity for cloud_resources table
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, SeaOrmResource)]
#[sea_orm(table_name = "cloud_resources")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub resource_type: String, // Stored as text, converted to/from enum
    pub status: String,        // Stored as text, converted to/from enum
    #[sea_orm(column_type = "Text")]
    pub region: String,
    pub configuration: Json, // JSONB field
    pub cost_per_hour: Option<f64>,
    pub monthly_cost_estimate: Option<f64>,
    pub tags: Json, // JSONB field
    pub enabled: bool,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
    pub deleted_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "domain_projects::entity::Entity",
        from = "Column::ProjectId",
        to = "domain_projects::entity::Column::Id"
    )]
    Projects,
}

impl Related<domain_projects::entity::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Projects.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

// Conversion from Sea-ORM Model to domain CloudResource.
// Fallible: `resource_type`/`status` are enum-backed String columns (no schema
// change), so a legacy/invalid row is surfaced as an error instead of panicking
// and taking down every `list()`.
impl TryFrom<Model> for crate::models::CloudResource {
    type Error = crate::error::CloudResourceError;

    fn try_from(model: Model) -> Result<Self, Self::Error> {
        let resource_type = model.resource_type.parse::<ResourceType>().map_err(|e| {
            crate::error::CloudResourceError::Internal(format!(
                "invalid resource_type '{}' in database: {e}",
                model.resource_type
            ))
        })?;
        let status = model.status.parse::<ResourceStatus>().map_err(|e| {
            crate::error::CloudResourceError::Internal(format!(
                "invalid status '{}' in database: {e}",
                model.status
            ))
        })?;

        let tags: Vec<Tag> = serde_json::from_value(model.tags.clone()).unwrap_or_else(|e| {
            tracing::warn!("Failed to parse cloud resource tags from JSON: {e}");
            Vec::new()
        });

        Ok(Self {
            id: model.id,
            project_id: model.project_id,
            name: model.name,
            resource_type,
            status,
            region: model.region,
            configuration: model.configuration,
            cost_per_hour: model.cost_per_hour,
            monthly_cost_estimate: model.monthly_cost_estimate,
            tags,
            enabled: model.enabled,
            created_at: model.created_at.into(),
            updated_at: model.updated_at.into(),
            deleted_at: model.deleted_at.map(|dt| dt.into()),
        })
    }
}

// Conversion from domain CreateCloudResource to Sea-ORM ActiveModel.
// Fallible: tag serialization is surfaced as a crate error rather than a panic.
impl TryFrom<crate::models::CreateCloudResource> for ActiveModel {
    type Error = crate::error::CloudResourceError;

    fn try_from(input: crate::models::CreateCloudResource) -> Result<Self, Self::Error> {
        let tags_json = serde_json::to_value(&input.tags).map_err(|e| {
            crate::error::CloudResourceError::Internal(format!("serialize tags: {e}"))
        })?;
        let monthly_cost = input.cost_per_hour.map(|hourly| hourly * 24.0 * 30.0);
        let now = chrono::Utc::now();

        Ok(ActiveModel {
            id: Set(Uuid::now_v7()),
            project_id: Set(input.project_id),
            name: Set(input.name),
            resource_type: Set(input.resource_type.to_string()),
            status: Set(ResourceStatus::Creating.to_string()),
            region: Set(input.region),
            configuration: Set(input.configuration),
            cost_per_hour: Set(input.cost_per_hour),
            monthly_cost_estimate: Set(monthly_cost),
            tags: Set(tags_json),
            enabled: Set(true),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            deleted_at: Set(None),
        })
    }
}
