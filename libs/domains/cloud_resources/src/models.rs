use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

/// Cloud resource type
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, ToSchema,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ResourceType {
    Compute,
    Storage,
    Database,
    Network,
    Serverless,
    Analytics,
    Other,
}

/// Resource status
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    Default,
    ToSchema,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ResourceStatus {
    #[default]
    Creating,
    Active,
    Updating,
    Deleting,
    Deleted,
    Failed,
}

/// Cloud resource entity
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CloudResource {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub resource_type: ResourceType,
    pub status: ResourceStatus,
    pub region: String,
    pub configuration: serde_json::Value,
    pub cost_per_hour: Option<f64>,
    pub monthly_cost_estimate: Option<f64>,
    pub tags: Vec<Tag>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Key-value tag for resource organization
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct Tag {
    pub key: String,
    pub value: String,
}

/// DTO for creating a new cloud resource
#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
pub struct CreateCloudResource {
    pub project_id: Uuid,
    #[validate(length(min = 1, max = 255))]
    pub name: String,
    pub resource_type: ResourceType,
    #[validate(length(min = 1))]
    pub region: String,
    #[serde(default)]
    pub configuration: serde_json::Value,
    #[validate(range(min = 0.0))]
    pub cost_per_hour: Option<f64>,
    #[serde(default)]
    pub tags: Vec<Tag>,
}

/// DTO for updating an existing cloud resource
#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
pub struct UpdateCloudResource {
    #[validate(length(min = 1, max = 255))]
    pub name: Option<String>,
    pub status: Option<ResourceStatus>,
    #[validate(length(min = 1))]
    pub region: Option<String>,
    pub configuration: Option<serde_json::Value>,
    #[validate(range(min = 0.0))]
    pub cost_per_hour: Option<f64>,
    pub monthly_cost_estimate: Option<f64>,
    pub tags: Option<Vec<Tag>>,
    pub enabled: Option<bool>,
}

/// Query filters for listing cloud resources
#[derive(Debug, Clone, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct CloudResourceFilter {
    pub project_id: Option<Uuid>,
    pub resource_type: Option<ResourceType>,
    pub status: Option<ResourceStatus>,
    pub region: Option<String>,
    pub enabled: Option<bool>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

impl Default for CloudResourceFilter {
    fn default() -> Self {
        Self {
            project_id: None,
            resource_type: None,
            status: None,
            region: None,
            enabled: None,
            limit: default_limit(),
            offset: 0,
        }
    }
}

impl CloudResource {
    /// Create a new cloud resource from CreateCloudResource DTO
    pub fn new(input: CreateCloudResource) -> Self {
        let now = Utc::now();
        let monthly_cost = input.cost_per_hour.map(|hourly| hourly * 24.0 * 30.0);

        Self {
            id: Uuid::now_v7(),
            project_id: input.project_id,
            name: input.name,
            resource_type: input.resource_type,
            status: ResourceStatus::Creating,
            region: input.region,
            configuration: input.configuration,
            cost_per_hour: input.cost_per_hour,
            monthly_cost_estimate: monthly_cost,
            tags: input.tags,
            enabled: true,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    /// Apply updates from UpdateCloudResource DTO
    pub fn apply_update(&mut self, update: UpdateCloudResource) {
        if let Some(name) = update.name {
            self.name = name;
        }
        if let Some(status) = update.status {
            self.status = status;
        }
        if let Some(region) = update.region {
            self.region = region;
        }
        if let Some(configuration) = update.configuration {
            self.configuration = configuration;
        }
        if let Some(cost_per_hour) = update.cost_per_hour {
            self.cost_per_hour = Some(cost_per_hour);
            self.monthly_cost_estimate = Some(cost_per_hour * 24.0 * 30.0);
        }
        if let Some(monthly_cost_estimate) = update.monthly_cost_estimate {
            self.monthly_cost_estimate = Some(monthly_cost_estimate);
        }
        if let Some(tags) = update.tags {
            self.tags = tags;
        }
        if let Some(enabled) = update.enabled {
            self.enabled = enabled;
        }
        self.updated_at = Utc::now();
    }

    /// Soft delete the resource
    pub fn soft_delete(&mut self) {
        self.status = ResourceStatus::Deleted;
        self.deleted_at = Some(Utc::now());
        self.updated_at = Utc::now();
    }
}
