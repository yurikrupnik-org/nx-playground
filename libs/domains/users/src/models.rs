use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

/// User roles
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    User,
    Admin,
    Moderator,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::User => write!(f, "user"),
            Role::Admin => write!(f, "admin"),
            Role::Moderator => write!(f, "moderator"),
        }
    }
}

impl std::str::FromStr for Role {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "user" => Ok(Role::User),
            "admin" => Ok(Role::Admin),
            "moderator" => Ok(Role::Moderator),
            _ => Err(format!("Unknown role: {s}")),
        }
    }
}

/// User entity - matches SQL schema
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct User {
    /// Unique identifier
    pub id: Uuid,
    /// User email (unique)
    pub email: String,
    /// User display name
    pub name: String,
    /// IdP subject (`sub`, e.g. WorkOS `user_...`) — set on first OIDC login
    pub subject: Option<String>,
    /// User roles
    pub roles: Vec<Role>,
    /// Whether email has been verified
    pub email_verified: bool,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
    /// Avatar URL (from the IdP or user upload)
    pub avatar_url: Option<String>,
    /// Last login timestamp
    pub last_login_at: Option<DateTime<Utc>>,
    /// Account active status
    pub is_active: bool,
}

/// User response DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub roles: Vec<String>,
    pub email_verified: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub avatar_url: Option<String>,
    pub last_login_at: Option<DateTime<Utc>>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            email: user.email,
            name: user.name,
            roles: user.roles.iter().map(|r| r.to_string()).collect(),
            email_verified: user.email_verified,
            created_at: user.created_at,
            updated_at: user.updated_at,
            avatar_url: user.avatar_url,
            last_login_at: user.last_login_at,
        }
    }
}

/// DTO for creating a new user
#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
pub struct CreateUser {
    #[validate(email, length(max = 255))]
    pub email: String,
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    #[serde(default)]
    pub roles: Vec<String>,
}

/// DTO for updating an existing user
#[derive(Debug, Clone, Default, Deserialize, Validate, ToSchema)]
pub struct UpdateUser {
    #[validate(email, length(max = 255))]
    pub email: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub name: Option<String>,
    pub roles: Option<Vec<String>>,
    pub email_verified: Option<bool>,
}

/// Query filters for listing users
#[derive(Debug, Clone, Default, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct UserFilter {
    pub email: Option<String>,
    pub role: Option<String>,
    pub email_verified: Option<bool>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

impl User {
    /// Create a new user (identity/credentials live at the IdP)
    pub fn new(email: String, name: String, roles: Vec<Role>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            email,
            name,
            subject: None,
            roles: if roles.is_empty() {
                vec![Role::User]
            } else {
                roles
            },
            email_verified: false,
            created_at: now,
            updated_at: now,
            avatar_url: None,
            last_login_at: None,
            is_active: true,
        }
    }

    /// Apply updates
    pub fn apply_update(&mut self, update: UpdateUser) {
        if let Some(email) = update.email {
            self.email = email;
        }
        if let Some(name) = update.name {
            self.name = name;
        }
        if let Some(roles) = update.roles {
            self.roles = roles.iter().filter_map(|r| r.parse().ok()).collect();
            if self.roles.is_empty() {
                self.roles = vec![Role::User];
            }
        }
        if let Some(verified) = update.email_verified {
            self.email_verified = verified;
        }
        self.updated_at = Utc::now();
    }
}
