use chrono::{DateTime, Utc};
use regex::Regex;
use sea_orm::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use strum::Display;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

/// Regex pattern for alphanumeric characters with hyphens and underscores
static ALPHANUMERIC_HYPHEN_UNDERSCORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_-]+$").expect("valid alphanumeric regex"));

/// Custom validator for project names
fn validate_project_name(name: &str) -> Result<(), validator::ValidationError> {
    if !ALPHANUMERIC_HYPHEN_UNDERSCORE.is_match(name) {
        return Err(validator::ValidationError::new("invalid_project_name"));
    }
    Ok(())
}

/// Supported cloud providers
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    DeriveActiveEnum,
    EnumIter,
    ToSchema,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "cloud_provider")]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum CloudProvider {
    #[sea_orm(string_value = "aws")]
    Aws,
    #[sea_orm(string_value = "gcp")]
    Gcp,
    #[sea_orm(string_value = "azure")]
    Azure,
    #[sea_orm(string_value = "local")]
    Local,
}

/// Project deployment status
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    Default,
    DeriveActiveEnum,
    EnumIter,
    ToSchema,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "project_status")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ProjectStatus {
    /// Project is being set up
    #[default]
    #[sea_orm(string_value = "provisioning")]
    Provisioning,
    /// Project is active and running
    #[sea_orm(string_value = "active")]
    Active,
    /// Project is temporarily suspended
    #[sea_orm(string_value = "suspended")]
    Suspended,
    /// Project is being deleted
    #[sea_orm(string_value = "deleting")]
    Deleting,
    /// Project has been archived
    #[sea_orm(string_value = "archived")]
    Archived,
}

/// Environment type for the project
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    Default,
    DeriveActiveEnum,
    EnumIter,
    ToSchema,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "environment")]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Environment {
    #[default]
    #[sea_orm(string_value = "development")]
    Development,
    #[sea_orm(string_value = "staging")]
    Staging,
    #[sea_orm(string_value = "production")]
    Production,
}

/// Project entity - represents a cloud project
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Project {
    /// Unique identifier
    pub id: Uuid,
    /// Project name (must be unique per user)
    pub name: String,
    /// Owner of the project
    pub user_id: Uuid,
    /// Project description
    pub description: String,
    /// Cloud provider (AWS, GCP, Azure)
    pub cloud_provider: CloudProvider,
    /// Deployment region (e.g., us-east-1, europe-west1)
    pub region: String,
    /// Environment type
    pub environment: Environment,
    /// Current status
    pub status: ProjectStatus,
    /// Optional monthly budget limit in USD
    pub budget_limit: Option<f64>,
    /// Resource tags for organization
    pub tags: Vec<Tag>,
    /// Whether the project is enabled
    pub enabled: bool,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Key-value tag for project organization
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Validate, ToSchema)]
pub struct Tag {
    #[validate(length(min = 1))]
    pub key: String,
    pub value: String,
}

/// DTO for creating a new project
#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
pub struct CreateProject {
    #[validate(length(min = 1, max = 100), custom(function = "validate_project_name"))]
    pub name: String,
    pub user_id: Uuid,
    #[serde(default)]
    pub description: String,
    pub cloud_provider: CloudProvider,
    #[validate(length(min = 1))]
    pub region: String,
    #[serde(default)]
    pub environment: Environment,
    #[validate(range(min = 0.0))]
    pub budget_limit: Option<f64>,
    #[serde(default)]
    #[validate(nested)]
    pub tags: Vec<Tag>,
}

/// DTO for updating an existing project
#[derive(Debug, Clone, Default, Deserialize, Validate, ToSchema)]
pub struct UpdateProject {
    #[validate(length(min = 1, max = 100), custom(function = "validate_project_name"))]
    pub name: Option<String>,
    pub description: Option<String>,
    pub region: Option<String>,
    pub environment: Option<Environment>,
    pub status: Option<ProjectStatus>,
    #[validate(range(min = 0.0))]
    pub budget_limit: Option<f64>,
    #[validate(nested)]
    pub tags: Option<Vec<Tag>>,
    pub enabled: Option<bool>,
}

/// Query filters for listing projects
#[derive(Debug, Clone, Deserialize, ToSchema, IntoParams)]
pub struct ProjectFilter {
    pub user_id: Option<Uuid>,
    pub cloud_provider: Option<CloudProvider>,
    pub environment: Option<Environment>,
    pub status: Option<ProjectStatus>,
    pub enabled: Option<bool>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

impl Default for ProjectFilter {
    fn default() -> Self {
        Self {
            user_id: None,
            cloud_provider: None,
            environment: None,
            status: None,
            enabled: None,
            limit: default_limit(),
            offset: 0,
        }
    }
}

impl Project {
    /// Create a new project from CreateProject DTO
    pub fn new(input: CreateProject) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            name: input.name,
            user_id: input.user_id,
            description: input.description,
            cloud_provider: input.cloud_provider,
            region: input.region,
            environment: input.environment,
            status: ProjectStatus::Provisioning,
            budget_limit: input.budget_limit,
            tags: input.tags,
            enabled: true,
            created_at: now,
            updated_at: now,
        }
    }

    /// Apply updates from UpdateProject DTO
    pub fn apply_update(&mut self, update: UpdateProject) {
        if let Some(name) = update.name {
            self.name = name;
        }
        if let Some(description) = update.description {
            self.description = description;
        }
        if let Some(region) = update.region {
            self.region = region;
        }
        if let Some(environment) = update.environment {
            self.environment = environment;
        }
        if let Some(status) = update.status {
            self.status = status;
        }
        if let Some(budget_limit) = update.budget_limit {
            self.budget_limit = Some(budget_limit);
        }
        if let Some(tags) = update.tags {
            self.tags = tags;
        }
        if let Some(enabled) = update.enabled {
            self.enabled = enabled;
        }
        self.updated_at = Utc::now();
    }
}
