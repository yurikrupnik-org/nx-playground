use std::sync::Arc;
use uuid::Uuid;

use crate::error::{UserError, UserResult};
use crate::models::{CreateUser, Role, UpdateUser, User, UserFilter, UserResponse};
use crate::repository::UserRepository;

/// Service layer for User business logic. Credentials live at the IdP (WorkOS);
/// this service only manages the local user rows.
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

    /// Create a new user
    pub async fn create_user(&self, input: CreateUser) -> UserResult<UserResponse> {
        let CreateUser { email, name, roles } = input;

        // Parse roles
        let roles: Vec<Role> = roles.iter().filter_map(|r| r.parse().ok()).collect();

        let user = User::new(email, name, roles);

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

    /// Get a user by IdP subject (`sub` claim)
    pub async fn get_user_by_subject(&self, subject: &str) -> UserResult<UserResponse> {
        let user = self
            .repository
            .get_by_subject(subject)
            .await?
            .ok_or_else(|| UserError::EmailNotFound(subject.to_string()))?;

        Ok(user.into())
    }

    /// Just-in-time provisioning for an IdP-authenticated principal:
    /// find by subject → link a pre-IdP account by email (backfilling `subject`) →
    /// create. Returns the user and whether it was newly created; always records
    /// the login timestamp.
    pub async fn provision_oidc_user(
        &self,
        subject: &str,
        email: &str,
        name: Option<&str>,
    ) -> UserResult<(UserResponse, bool)> {
        if let Some(user) = self.repository.get_by_subject(subject).await? {
            self.repository.touch_last_login(user.id).await?;
            return Ok((user.into(), false));
        }
        if let Some(user) = self.repository.get_by_email(email).await? {
            // Pre-IdP account with the same email: link it to the IdP identity.
            self.repository.set_subject(user.id, subject).await?;
            self.repository.touch_last_login(user.id).await?;
            return Ok((user.into(), false));
        }
        let mut user = User::new(
            email.to_string(),
            name.unwrap_or(email).to_string(),
            vec![Role::User],
        );
        user.subject = Some(subject.to_string());
        // The IdP owns credentials and verifies addresses before releasing tokens.
        user.email_verified = true;
        user.last_login_at = Some(chrono::Utc::now());
        let created = self.repository.create(user).await?;
        Ok((created.into(), true))
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
        // Get existing user
        let mut user = self
            .repository
            .get_by_id(id)
            .await?
            .ok_or(UserError::NotFound(id))?;

        // Check for duplicate email if email is being changed
        if let Some(new_email) = &input.email
            && !new_email.eq_ignore_ascii_case(&user.email)
            && self.repository.email_exists(new_email).await?
        {
            return Err(UserError::DuplicateEmail(new_email.clone()));
        }

        user.apply_update(input);

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
}
