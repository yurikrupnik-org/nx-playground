use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::{UserError, UserResult};
use crate::models::{User, UserFilter};

/// Repository trait for User persistence
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Create a new user
    async fn create(&self, user: User) -> UserResult<User>;

    /// Get a user by ID
    async fn get_by_id(&self, id: Uuid) -> UserResult<Option<User>>;

    /// Get a user by email
    async fn get_by_email(&self, email: &str) -> UserResult<Option<User>>;

    /// Get a user by IdP subject (`sub` claim)
    async fn get_by_subject(&self, subject: &str) -> UserResult<Option<User>>;

    /// Backfill the IdP subject on an existing user (links a pre-IdP account)
    async fn set_subject(&self, user_id: Uuid, subject: &str) -> UserResult<()>;

    /// Record a successful login
    async fn touch_last_login(&self, user_id: Uuid) -> UserResult<()>;

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

    async fn get_by_subject(&self, subject: &str) -> UserResult<Option<User>> {
        let users = self.users.read().await;
        let user = users
            .values()
            .find(|u| u.subject.as_deref() == Some(subject))
            .cloned();
        Ok(user)
    }

    async fn set_subject(&self, user_id: Uuid, subject: &str) -> UserResult<()> {
        let mut users = self.users.write().await;
        let user = users
            .get_mut(&user_id)
            .ok_or(UserError::NotFound(user_id))?;
        user.subject = Some(subject.to_string());
        user.updated_at = chrono::Utc::now();
        Ok(())
    }

    async fn touch_last_login(&self, user_id: Uuid) -> UserResult<()> {
        let mut users = self.users.write().await;
        let user = users
            .get_mut(&user_id)
            .ok_or(UserError::NotFound(user_id))?;
        user.last_login_at = Some(chrono::Utc::now());
        user.updated_at = chrono::Utc::now();
        Ok(())
    }

    async fn list(&self, filter: UserFilter) -> UserResult<Vec<User>> {
        let users = self.users.read().await;

        let mut result: Vec<User> = users
            .values()
            .filter(|u| {
                if let Some(email) = &filter.email
                    && !u.email.to_lowercase().contains(&email.to_lowercase())
                {
                    return false;
                }
                if let Some(role) = &filter.role
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
        result.sort_by_key(|b| std::cmp::Reverse(b.created_at));

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
                if let Some(email) = &filter.email
                    && !u.email.to_lowercase().contains(&email.to_lowercase())
                {
                    return false;
                }
                if let Some(role) = &filter.role
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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::models::Role;

    #[tokio::test]
    async fn test_create_and_get_user() {
        let repo = InMemoryUserRepository::new();

        let user = User::new(
            "test@example.com".to_string(),
            "Test User".to_string(),
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
            vec![Role::User],
        );

        let user2 = User::new(
            "test@example.com".to_string(),
            "User 2".to_string(),
            vec![Role::User],
        );

        repo.create(user1).await.unwrap();

        let result = repo.create(user2).await;
        assert!(matches!(result, Err(UserError::DuplicateEmail(_))));
    }

    #[tokio::test]
    async fn test_subject_lookup_and_backfill() {
        let repo = InMemoryUserRepository::new();

        let user = User::new(
            "test@example.com".to_string(),
            "Test User".to_string(),
            vec![Role::User],
        );
        let created = repo.create(user).await.unwrap();

        // No subject yet.
        assert!(
            repo.get_by_subject("user_123").await.unwrap().is_none(),
            "unlinked subject must not resolve"
        );

        // Backfill and resolve.
        repo.set_subject(created.id, "user_123").await.unwrap();
        let linked = repo.get_by_subject("user_123").await.unwrap().unwrap();
        assert_eq!(linked.id, created.id);

        // Login stamp.
        repo.touch_last_login(created.id).await.unwrap();
        let user = repo.get_by_id(created.id).await.unwrap().unwrap();
        assert!(user.last_login_at.is_some());
    }
}
