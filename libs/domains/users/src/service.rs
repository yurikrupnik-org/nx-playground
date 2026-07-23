use std::sync::Arc;
use uuid::Uuid;

use crate::error::{UserError, UserResult};
use crate::models::{CreateUser, Role, UpdateUser, User, UserFilter, UserResponse};
use crate::password;
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
        Self::validate_create(&input)?;

        let CreateUser {
            email,
            name,
            password,
            roles,
        } = input;

        // Hash password
        let password_hash = password::hash_password(password).await?;

        // Parse roles
        let roles: Vec<Role> = roles.iter().filter_map(|r| r.parse().ok()).collect();

        let user = User::new(email, name, password_hash, roles);

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
        let user = self
            .repository
            .get_by_email(email)
            .await?
            .ok_or_else(|| UserError::EmailNotFound(email.to_string()))?;

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
    pub async fn update_user(&self, id: Uuid, mut input: UpdateUser) -> UserResult<UserResponse> {
        // Validate input
        Self::validate_update(&input)?;

        // Get existing user
        let mut user = self
            .repository
            .get_by_id(id)
            .await?
            .ok_or(UserError::NotFound(id))?;

        // Hash new password if provided
        let new_password_hash = match input.password.take() {
            Some(new_password) => Some(password::hash_password(new_password).await?),
            None => None,
        };

        // Check for duplicate email if email is being changed
        if let Some(new_email) = &input.email
            && !new_email.eq_ignore_ascii_case(&user.email)
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
                "Account is locked until {locked_until}"
            )));
        }

        // Verify password
        if !password::verify_password(password.to_string(), user.password_hash.clone()).await? {
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
        if !password::verify_password(current_password.to_string(), user.password_hash.clone())
            .await?
        {
            return Err(UserError::InvalidCredentials);
        }

        // Validate new password
        Self::validate_password(new_password)?;

        // Hash and update
        user.password_hash = password::hash_password(new_password.to_string()).await?;
        user.updated_at = chrono::Utc::now();

        self.repository.update(user).await?;
        Ok(())
    }

    // Validation helpers

    // Email and name validation is now handled by ValidatedJson<T> at the handler level
    // using the validator crate with #[validate(email)] and #[validate(length(...))] attributes

    fn validate_create(input: &CreateUser) -> UserResult<()> {
        Self::validate_password(&input.password)?;
        Ok(())
    }

    fn validate_update(input: &UpdateUser) -> UserResult<()> {
        if let Some(password) = &input.password {
            Self::validate_password(password)?;
        }
        Ok(())
    }

    fn validate_password(password: &str) -> UserResult<()> {
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
}
