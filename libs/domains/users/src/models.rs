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
            _ => Err(format!("Unknown role: {}", s)),
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
    /// Argon2 password hash (never exposed in API responses)
    #[serde(skip_serializing)]
    pub password_hash: String,
    /// User roles
    pub roles: Vec<Role>,
    /// Whether email has been verified
    pub email_verified: bool,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
    /// Avatar URL (from OAuth or user upload)
    pub avatar_url: Option<String>,
    /// Last login timestamp
    pub last_login_at: Option<DateTime<Utc>>,
    /// Account active status
    pub is_active: bool,
    /// Account locked status
    pub is_locked: bool,
    /// Failed login attempt counter
    pub failed_login_attempts: i32,
    /// Locked until timestamp
    pub locked_until: Option<DateTime<Utc>>,
}

/// User response DTO (without password_hash)
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
    pub password: String,
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
    pub password: Option<String>,
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

/// DTO for user login
#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
pub struct LoginRequest {
    #[validate(email, length(max = 255))]
    pub email: String,
    pub password: String,
}

/// DTO for user registration
#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
pub struct RegisterRequest {
    #[validate(email, length(max = 255))]
    pub email: String,
    pub password: String,
    #[validate(length(min = 1, max = 100))]
    pub name: String,
}

/// Response after successful login/register
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LoginResponse {
    pub user: UserResponse,
}

impl User {
    /// Create a new user (password will be hashed by service layer)
    pub fn new(email: String, name: String, password_hash: String, roles: Vec<Role>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            email,
            name,
            password_hash,
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
            is_locked: false,
            failed_login_attempts: 0,
            locked_until: None,
        }
    }

    /// Apply updates (password should already be hashed if provided)
    pub fn apply_update(&mut self, update: UpdateUser, new_password_hash: Option<String>) {
        if let Some(email) = update.email {
            self.email = email;
        }
        if let Some(name) = update.name {
            self.name = name;
        }
        if let Some(hash) = new_password_hash {
            self.password_hash = hash;
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
