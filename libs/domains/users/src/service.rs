use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{UserError, UserResult};
use crate::models::{CreateUser, Role, UpdateUser, User, UserFilter, UserResponse};
use crate::oauth::{OAuthUserInfo, Provider};
use crate::repository::UserRepository;

/// Service layer for User business logic
#[derive(Clone)]
pub struct UserService<R: UserRepository> {
    repository: Arc<R>,
}

impl<R: UserRepository> UserService<R> {
    pub fn new(repository: R) -> Self {
        Self {
            repository: Arc::new(repository),
        }
    }

    /// Create a new user with password hashing
    pub async fn create_user(&self, input: CreateUser) -> UserResult<UserResponse> {
        // Validate input
        self.validate_create(&input)?;

        // Hash password
        let password_hash = self.hash_password(&input.password)?;

        // Parse roles
        let roles: Vec<Role> = input.roles.iter().filter_map(|r| r.parse().ok()).collect();

        let user = User::new(input.email, input.name, password_hash, roles);

        let created = self.repository.create(user).await?;
        Ok(created.into())
    }

    /// Get a user by ID
    pub async fn get_user(&self, id: Uuid) -> UserResult<UserResponse> {
        let user = self
            .repository
            .get_by_id(id)
            .await?
            .ok_or(UserError::NotFound(id))?;

        Ok(user.into())
    }

    /// Get a user by email
    pub async fn get_user_by_email(&self, email: &str) -> UserResult<UserResponse> {
        let user = self.repository.get_by_email(email).await?.ok_or_else(|| {
            UserError::Validation(format!("User with email '{}' not found", email))
        })?;

        Ok(user.into())
    }

    /// List users with filters
    pub async fn list_users(&self, filter: UserFilter) -> UserResult<(Vec<UserResponse>, usize)> {
        let total = self.repository.count(filter.clone()).await?;
        let users = self.repository.list(filter).await?;
        let responses: Vec<UserResponse> = users.into_iter().map(|u| u.into()).collect();
        Ok((responses, total))
    }

    /// Update a user
    pub async fn update_user(&self, id: Uuid, input: UpdateUser) -> UserResult<UserResponse> {
        // Validate input
        self.validate_update(&input)?;

        // Get existing user
        let mut user = self
            .repository
            .get_by_id(id)
            .await?
            .ok_or(UserError::NotFound(id))?;

        // Hash new password if provided
        let new_password_hash = if let Some(ref password) = input.password {
            Some(self.hash_password(password)?)
        } else {
            None
        };

        // Check for duplicate email if email is being changed
        if let Some(ref new_email) = input.email
            && new_email.to_lowercase() != user.email.to_lowercase()
            && self.repository.email_exists(new_email).await?
        {
            return Err(UserError::DuplicateEmail(new_email.clone()));
        }

        user.apply_update(input, new_password_hash);

        let updated = self.repository.update(user).await?;
        Ok(updated.into())
    }

    /// Delete a user
    pub async fn delete_user(&self, id: Uuid) -> UserResult<()> {
        let deleted = self.repository.delete(id).await?;

        if !deleted {
            return Err(UserError::NotFound(id));
        }

        Ok(())
    }

    /// Verify user credentials (for login)
    pub async fn verify_credentials(
        &self,
        email: &str,
        password: &str,
    ) -> UserResult<UserResponse> {
        let user = self
            .repository
            .get_by_email(email)
            .await?
            .ok_or(UserError::InvalidCredentials)?;

        // Check if account is active
        if !user.is_active {
            return Err(UserError::Validation("Account is inactive".to_string()));
        }

        // Check if account is locked
        if self.repository.check_account_locked(user.id).await? {
            let locked_until = user
                .locked_until
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| "unknown".to_string());
            return Err(UserError::Validation(format!(
                "Account is locked until {}",
                locked_until
            )));
        }

        // Verify password
        if !self.verify_password(password, &user.password_hash)? {
            // Increment failed login attempts
            self.repository.update_login_attempt(user.id, false).await?;
            return Err(UserError::InvalidCredentials);
        }

        // Successful login - reset failed attempts and update last login
        self.repository.update_login_attempt(user.id, true).await?;

        Ok(user.into())
    }

    /// Verify email (mark as verified)
    pub async fn verify_email(&self, id: Uuid) -> UserResult<UserResponse> {
        let mut user = self
            .repository
            .get_by_id(id)
            .await?
            .ok_or(UserError::NotFound(id))?;

        user.email_verified = true;
        user.updated_at = chrono::Utc::now();

        let updated = self.repository.update(user).await?;
        Ok(updated.into())
    }

    /// Change user password
    pub async fn change_password(
        &self,
        id: Uuid,
        current_password: &str,
        new_password: &str,
    ) -> UserResult<()> {
        let mut user = self
            .repository
            .get_by_id(id)
            .await?
            .ok_or(UserError::NotFound(id))?;

        // Verify the current password
        if !self.verify_password(current_password, &user.password_hash)? {
            return Err(UserError::InvalidCredentials);
        }

        // Validate new password
        self.validate_password(new_password)?;

        // Hash and update
        user.password_hash = self.hash_password(new_password)?;
        user.updated_at = chrono::Utc::now();

        self.repository.update(user).await?;
        Ok(())
    }

    // Password helpers

    fn hash_password(&self, password: &str) -> UserResult<String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| UserError::PasswordHash(e.to_string()))
    }

    fn verify_password(&self, password: &str, hash: &str) -> UserResult<bool> {
        let parsed_hash =
            PasswordHash::new(hash).map_err(|e| UserError::PasswordHash(e.to_string()))?;

        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    // Validation helpers

    // Email and name validation is now handled by ValidatedJson<T> at the handler level
    // using the validator crate with #[validate(email)] and #[validate(length(...))] attributes

    fn validate_create(&self, input: &CreateUser) -> UserResult<()> {
        self.validate_password(&input.password)?;
        Ok(())
    }

    fn validate_update(&self, input: &UpdateUser) -> UserResult<()> {
        if let Some(ref password) = input.password {
            self.validate_password(password)?;
        }
        Ok(())
    }

    fn validate_password(&self, password: &str) -> UserResult<()> {
        if password.len() < 8 {
            return Err(UserError::Validation(
                "Password must be at least 8 characters".to_string(),
            ));
        }

        if password.len() > 128 {
            return Err(UserError::Validation(
                "Password cannot exceed 128 characters".to_string(),
            ));
        }

        // Check for at least one uppercase letter
        if !password.chars().any(|c| c.is_uppercase()) {
            return Err(UserError::Validation(
                "Password must contain at least one uppercase letter".to_string(),
            ));
        }

        // Check for at least one lowercase letter
        if !password.chars().any(|c| c.is_lowercase()) {
            return Err(UserError::Validation(
                "Password must contain at least one lowercase letter".to_string(),
            ));
        }

        // Check for at least one digit
        if !password.chars().any(|c| c.is_numeric()) {
            return Err(UserError::Validation(
                "Password must contain at least one digit".to_string(),
            ));
        }

        // Check for at least one special character
        let special_chars = "!@#$%^&*()_+-=[]{}|;:,.<>?";
        if !password.chars().any(|c| special_chars.contains(c)) {
            return Err(UserError::Validation(
                "Password must contain at least one special character (!@#$%^&*()_+-=[]{}|;:,.<>?)"
                    .to_string(),
            ));
        }

        Ok(())
    }

    // OAuth methods

    /// Create a new user from OAuth information
    pub async fn create_user_from_oauth(
        &self,
        oauth_info: OAuthUserInfo,
        provider: Provider,
    ) -> UserResult<UserResponse> {
        // Generate a random password (won't be used since OAuth users don't use passwords)
        let random_password = uuid::Uuid::new_v4().to_string();
        let password_hash = self.hash_password(&random_password)?;

        let mut user = User::new(
            oauth_info
                .email
                .clone()
                .unwrap_or_else(|| "noemail@oauth.local".to_string()),
            oauth_info
                .name
                .clone()
                .unwrap_or_else(|| "OAuth User".to_string()),
            password_hash,
            vec![Role::User],
        );

        // Set avatar from OAuth
        user.avatar_url = oauth_info.avatar_url;

        // Mark email as verified (trust OAuth provider)
        user.email_verified = true;

        let created = self.repository.create(user).await?;

        // Link OAuth account via oauth_accounts table
        self.repository
            .link_oauth_account(created.id, provider, &oauth_info.provider_user_id, None)
            .await?;

        Ok(created.into())
    }

    /// Get user by OAuth provider ID
    pub async fn get_user_by_oauth_id(
        &self,
        provider: Provider,
        provider_id: &str,
    ) -> UserResult<Option<UserResponse>> {
        let user = self
            .repository
            .get_by_oauth_id(provider, provider_id)
            .await?;
        Ok(user.map(|u| u.into()))
    }

    /// Link OAuth account to an existing user
    pub async fn link_oauth_to_user(
        &self,
        user_id: Uuid,
        oauth_info: OAuthUserInfo,
        provider: Provider,
    ) -> UserResult<()> {
        self.repository
            .link_oauth_account(
                user_id,
                provider,
                &oauth_info.provider_user_id,
                oauth_info.avatar_url,
            )
            .await
    }
}
