use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::{UserError, UserResult};
use crate::models::{User, UserFilter};
use crate::oauth::Provider;

/// Repository trait for User persistence
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Create a new user
    async fn create(&self, user: User) -> UserResult<User>;

    /// Get a user by ID
    async fn get_by_id(&self, id: Uuid) -> UserResult<Option<User>>;

    /// Get a user by email
    async fn get_by_email(&self, email: &str) -> UserResult<Option<User>>;

    /// List users with optional filters
    async fn list(&self, filter: UserFilter) -> UserResult<Vec<User>>;

    /// Update an existing user
    async fn update(&self, user: User) -> UserResult<User>;

    /// Delete a user by ID
    async fn delete(&self, id: Uuid) -> UserResult<bool>;

    /// Check if an email already exists
    async fn email_exists(&self, email: &str) -> UserResult<bool>;

    /// Count total users (for pagination)
    async fn count(&self, filter: UserFilter) -> UserResult<usize>;

    /// Get a user by OAuth provider ID
    async fn get_by_oauth_id(
        &self,
        provider: Provider,
        provider_id: &str,
    ) -> UserResult<Option<User>>;

    /// Link OAuth account to an existing user
    async fn link_oauth_account(
        &self,
        user_id: Uuid,
        provider: Provider,
        provider_id: &str,
        avatar_url: Option<String>,
    ) -> UserResult<()>;

    /// Update login attempt (increment or reset)
    async fn update_login_attempt(&self, user_id: Uuid, success: bool) -> UserResult<()>;

    /// Check if account is currently locked
    async fn check_account_locked(&self, user_id: Uuid) -> UserResult<bool>;
}

/// In-memory implementation of UserRepository (for development/testing)
#[derive(Debug, Default, Clone)]
pub struct InMemoryUserRepository {
    users: Arc<RwLock<HashMap<Uuid, User>>>,
}

impl InMemoryUserRepository {
    pub fn new() -> Self {
        Self {
            users: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl UserRepository for InMemoryUserRepository {
    async fn create(&self, user: User) -> UserResult<User> {
        let mut users = self.users.write().await;

        // Check for duplicate email
        let email_exists = users
            .values()
            .any(|u| u.email.to_lowercase() == user.email.to_lowercase());

        if email_exists {
            return Err(UserError::DuplicateEmail(user.email));
        }

        users.insert(user.id, user.clone());

        tracing::info!(user_id = %user.id, email = %user.email, "Created user");
        Ok(user)
    }

    async fn get_by_id(&self, id: Uuid) -> UserResult<Option<User>> {
        let users = self.users.read().await;
        Ok(users.get(&id).cloned())
    }

    async fn get_by_email(&self, email: &str) -> UserResult<Option<User>> {
        let users = self.users.read().await;
        let user = users
            .values()
            .find(|u| u.email.to_lowercase() == email.to_lowercase())
            .cloned();
        Ok(user)
    }

    async fn list(&self, filter: UserFilter) -> UserResult<Vec<User>> {
        let users = self.users.read().await;

        let mut result: Vec<User> = users
            .values()
            .filter(|u| {
                if let Some(ref email) = filter.email
                    && !u.email.to_lowercase().contains(&email.to_lowercase())
                {
                    return false;
                }
                if let Some(ref role) = filter.role
                    && !u.roles.iter().any(|r| r.to_string() == *role)
                {
                    return false;
                }
                if let Some(verified) = filter.email_verified
                    && u.email_verified != verified
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();

        // Sort by created_at descending (newest first)
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        // Apply pagination
        let result: Vec<User> = result
            .into_iter()
            .skip(filter.offset)
            .take(filter.limit)
            .collect();

        Ok(result)
    }

    async fn update(&self, user: User) -> UserResult<User> {
        let mut users = self.users.write().await;

        // Check if user exists
        if !users.contains_key(&user.id) {
            return Err(UserError::NotFound(user.id));
        }

        // Check for duplicate email (excluding current user)
        let email_exists = users
            .values()
            .any(|u| u.id != user.id && u.email.to_lowercase() == user.email.to_lowercase());

        if email_exists {
            return Err(UserError::DuplicateEmail(user.email));
        }

        users.insert(user.id, user.clone());

        tracing::info!(user_id = %user.id, "Updated user");
        Ok(user)
    }

    async fn delete(&self, id: Uuid) -> UserResult<bool> {
        let mut users = self.users.write().await;

        if users.remove(&id).is_some() {
            tracing::info!(user_id = %id, "Deleted user");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn email_exists(&self, email: &str) -> UserResult<bool> {
        let users = self.users.read().await;
        let exists = users
            .values()
            .any(|u| u.email.to_lowercase() == email.to_lowercase());
        Ok(exists)
    }

    async fn count(&self, filter: UserFilter) -> UserResult<usize> {
        let users = self.users.read().await;

        let count = users
            .values()
            .filter(|u| {
                if let Some(ref email) = filter.email
                    && !u.email.to_lowercase().contains(&email.to_lowercase())
                {
                    return false;
                }
                if let Some(ref role) = filter.role
                    && !u.roles.iter().any(|r| r.to_string() == *role)
                {
                    return false;
                }
                if let Some(verified) = filter.email_verified
                    && u.email_verified != verified
                {
                    return false;
                }
                true
            })
            .count();

        Ok(count)
    }

    async fn get_by_oauth_id(
        &self,
        _provider: Provider,
        _provider_id: &str,
    ) -> UserResult<Option<User>> {
        // In-memory impl doesn't track oauth_accounts separately
        Ok(None)
    }

    async fn link_oauth_account(
        &self,
        user_id: Uuid,
        _provider: Provider,
        _provider_id: &str,
        avatar_url: Option<String>,
    ) -> UserResult<()> {
        let mut users = self.users.write().await;
        if let Some(user) = users.get_mut(&user_id) {
            if avatar_url.is_some() {
                user.avatar_url = avatar_url;
            }
            user.updated_at = chrono::Utc::now();
            Ok(())
        } else {
            Err(UserError::NotFound(user_id))
        }
    }

    async fn update_login_attempt(&self, user_id: Uuid, success: bool) -> UserResult<()> {
        let mut users = self.users.write().await;
        if let Some(user) = users.get_mut(&user_id) {
            if success {
                // Reset on successful login
                user.failed_login_attempts = 0;
                user.is_locked = false;
                user.locked_until = None;
                user.last_login_at = Some(chrono::Utc::now());
            } else {
                // Increment failed attempts
                user.failed_login_attempts += 1;

                // Lock account after 5 failed attempts
                if user.failed_login_attempts >= 5 {
                    user.is_locked = true;
                    user.locked_until = Some(chrono::Utc::now() + chrono::Duration::minutes(15));
                }
            }
            user.updated_at = chrono::Utc::now();
            Ok(())
        } else {
            Err(UserError::NotFound(user_id))
        }
    }

    async fn check_account_locked(&self, user_id: Uuid) -> UserResult<bool> {
        let users = self.users.read().await;
        if let Some(user) = users.get(&user_id) {
            if !user.is_locked {
                return Ok(false);
            }

            // Check if lock has expired
            if let Some(locked_until) = user.locked_until {
                Ok(locked_until > chrono::Utc::now())
            } else {
                Ok(user.is_locked)
            }
        } else {
            Err(UserError::NotFound(user_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Role;

    #[tokio::test]
    async fn test_create_and_get_user() {
        let repo = InMemoryUserRepository::new();

        let user = User::new(
            "test@example.com".to_string(),
            "Test User".to_string(),
            "hashed_password".to_string(),
            vec![Role::User],
        );

        let created = repo.create(user.clone()).await.unwrap();
        assert_eq!(created.email, "test@example.com");

        let fetched = repo.get_by_id(created.id).await.unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().id, created.id);
    }

    #[tokio::test]
    async fn test_get_by_email() {
        let repo = InMemoryUserRepository::new();

        let user = User::new(
            "test@example.com".to_string(),
            "Test User".to_string(),
            "hashed_password".to_string(),
            vec![Role::User],
        );

        repo.create(user).await.unwrap();

        let fetched = repo.get_by_email("test@example.com").await.unwrap();
        assert!(fetched.is_some());

        let fetched = repo.get_by_email("TEST@EXAMPLE.COM").await.unwrap();
        assert!(fetched.is_some()); // Case insensitive
    }

    #[tokio::test]
    async fn test_duplicate_email_error() {
        let repo = InMemoryUserRepository::new();

        let user1 = User::new(
            "test@example.com".to_string(),
            "User 1".to_string(),
            "hash1".to_string(),
            vec![Role::User],
        );

        let user2 = User::new(
            "test@example.com".to_string(),
            "User 2".to_string(),
            "hash2".to_string(),
            vec![Role::User],
        );

        repo.create(user1).await.unwrap();

        let result = repo.create(user2).await;
        assert!(matches!(result, Err(UserError::DuplicateEmail(_))));
    }
}
