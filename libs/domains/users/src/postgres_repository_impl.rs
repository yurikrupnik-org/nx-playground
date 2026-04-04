use super::{User, UserError, UserFilter, UserRepository, UserResult};
use crate::oauth::Provider;
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DbBackend, FromQueryResult, Statement};
use uuid::Uuid;

/// PostgreSQL implementation of UserRepository using SeaORM
#[derive(Clone)]
pub struct PostgresUserRepository {
    db: sea_orm::DatabaseConnection,
}

impl PostgresUserRepository {
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self { db }
    }
}

/// Helper struct for deserializing user rows from the database
#[derive(Debug, FromQueryResult)]
struct UserRow {
    id: Uuid,
    email: String,
    name: String,
    password_hash: String,
    roles: Vec<String>, // PostgreSQL text array
    email_verified: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    avatar_url: Option<String>,
    last_login_at: Option<chrono::DateTime<chrono::Utc>>,
    is_active: bool,
    is_locked: bool,
    failed_login_attempts: i32,
    locked_until: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        use crate::models::Role;
        use std::str::FromStr;

        // Convert Vec<String> back to Vec<Role>
        let roles = row
            .roles
            .iter()
            .filter_map(|s| Role::from_str(s).ok())
            .collect();

        User {
            id: row.id,
            email: row.email,
            name: row.name,
            password_hash: row.password_hash,
            roles,
            email_verified: row.email_verified,
            created_at: row.created_at,
            updated_at: row.updated_at,
            avatar_url: row.avatar_url,
            last_login_at: row.last_login_at,
            is_active: row.is_active,
            is_locked: row.is_locked,
            failed_login_attempts: row.failed_login_attempts,
            locked_until: row.locked_until,
        }
    }
}

#[async_trait]
impl UserRepository for PostgresUserRepository {
    async fn create(&self, user: User) -> UserResult<User> {
        let sql = r#"
            INSERT INTO users (id, email, name, password_hash, roles, email_verified, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
        "#;

        // Convert roles Vec<Role> to Vec<String> for PostgreSQL text array
        let roles_array: Vec<String> = user.roles.iter().map(|r| r.to_string()).collect();

        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [
                user.id.into(),
                user.email.clone().into(),
                user.name.clone().into(),
                user.password_hash.clone().into(),
                roles_array.into(),
                user.email_verified.into(),
                user.created_at.into(),
                user.updated_at.into(),
            ],
        );

        let row = UserRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("duplicate key") || err_str.contains("unique constraint") {
                    UserError::DuplicateEmail(user.email.clone())
                } else {
                    UserError::Internal(format!("Database error: {}", e))
                }
            })?
            .ok_or_else(|| UserError::Internal("Failed to create user".to_string()))?;

        Ok(row.into())
    }

    async fn get_by_id(&self, id: Uuid) -> UserResult<Option<User>> {
        let sql = "SELECT * FROM users WHERE id = $1";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [id.into()]);

        let row = UserRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    async fn get_by_email(&self, email: &str) -> UserResult<Option<User>> {
        let sql = "SELECT * FROM users WHERE email = $1";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [email.into()]);

        let row = UserRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    async fn list(&self, _filter: UserFilter) -> UserResult<Vec<User>> {
        let sql = "SELECT * FROM users ORDER BY created_at DESC";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, []);

        let rows = UserRow::find_by_statement(stmt)
            .all(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn update(&self, user: User) -> UserResult<User> {
        let sql = r#"
            UPDATE users
            SET email = $2, name = $3, password_hash = $4, roles = $5,
                email_verified = $6, updated_at = $7, avatar_url = $8, last_login_at = $9,
                is_active = $10, is_locked = $11,
                failed_login_attempts = $12, locked_until = $13
            WHERE id = $1
            RETURNING *
        "#;

        // Convert roles Vec<Role> to Vec<String> for PostgreSQL text array
        let roles_array: Vec<String> = user.roles.iter().map(|r| r.to_string()).collect();

        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [
                user.id.into(),
                user.email.clone().into(),
                user.name.clone().into(),
                user.password_hash.clone().into(),
                roles_array.into(),
                user.email_verified.into(),
                user.updated_at.into(),
                user.avatar_url.clone().into(),
                user.last_login_at.into(),
                user.is_active.into(),
                user.is_locked.into(),
                user.failed_login_attempts.into(),
                user.locked_until.into(),
            ],
        );

        let row = UserRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        row.map(|r| r.into()).ok_or(UserError::NotFound(user.id))
    }

    async fn delete(&self, id: Uuid) -> UserResult<bool> {
        let sql = "DELETE FROM users WHERE id = $1";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [id.into()]);

        let result = self
            .db
            .execute_raw(stmt)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_by_oauth_id(
        &self,
        provider: Provider,
        provider_id: &str,
    ) -> UserResult<Option<User>> {
        let sql = r#"
            SELECT u.* FROM users u
            INNER JOIN oauth_accounts oa ON oa.user_id = u.id
            WHERE oa.provider = $1 AND oa.provider_user_id = $2
        "#;

        let provider_str = match provider {
            Provider::Google => "google",
            Provider::Github => "github",
        };

        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [provider_str.into(), provider_id.into()],
        );

        let row = UserRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    async fn link_oauth_account(
        &self,
        user_id: Uuid,
        provider: Provider,
        provider_id: &str,
        avatar_url: Option<String>,
    ) -> UserResult<()> {
        let provider_str = match provider {
            Provider::Google => "google",
            Provider::Github => "github",
        };

        // Upsert into oauth_accounts
        let sql = r#"
            INSERT INTO oauth_accounts (user_id, provider, provider_user_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (provider, provider_user_id) DO NOTHING
        "#;

        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [user_id.into(), provider_str.into(), provider_id.into()],
        );

        self.db
            .execute_raw(stmt)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        // Update avatar if provided
        if let Some(url) = avatar_url {
            let avatar_sql =
                "UPDATE users SET avatar_url = $2 WHERE id = $1 AND avatar_url IS NULL";
            let avatar_stmt = Statement::from_sql_and_values(
                DbBackend::Postgres,
                avatar_sql,
                [user_id.into(), url.into()],
            );
            self.db
                .execute_raw(avatar_stmt)
                .await
                .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;
        }

        Ok(())
    }

    async fn email_exists(&self, email: &str) -> UserResult<bool> {
        let sql = "SELECT EXISTS(SELECT 1 FROM users WHERE email = $1) as exists";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [email.into()]);

        #[derive(FromQueryResult)]
        struct ExistsResult {
            exists: bool,
        }

        let result = ExistsResult::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        Ok(result.map(|r| r.exists).unwrap_or(false))
    }

    async fn count(&self, _filter: UserFilter) -> UserResult<usize> {
        let sql = "SELECT COUNT(*) as count FROM users";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, []);

        #[derive(FromQueryResult)]
        struct CountResult {
            count: i64,
        }

        let result = CountResult::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        Ok(result.map(|r| r.count as usize).unwrap_or(0))
    }

    async fn update_login_attempt(&self, user_id: Uuid, success: bool) -> UserResult<()> {
        let sql = if success {
            "UPDATE users SET failed_login_attempts = 0, last_login_at = NOW() WHERE id = $1"
        } else {
            r#"
                UPDATE users
                SET failed_login_attempts = failed_login_attempts + 1,
                    is_locked = CASE WHEN failed_login_attempts + 1 >= 5 THEN true ELSE is_locked END,
                    locked_until = CASE WHEN failed_login_attempts + 1 >= 5 THEN NOW() + INTERVAL '15 minutes' ELSE locked_until END
                WHERE id = $1
            "#
        };

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [user_id.into()]);

        self.db
            .execute_raw(stmt)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        Ok(())
    }

    async fn check_account_locked(&self, user_id: Uuid) -> UserResult<bool> {
        let sql = "SELECT is_locked, locked_until FROM users WHERE id = $1";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [user_id.into()]);

        #[derive(FromQueryResult)]
        struct LockStatus {
            is_locked: bool,
            locked_until: Option<chrono::DateTime<chrono::Utc>>,
        }

        let row = LockStatus::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;

        if let Some(lock_status) = row {
            if !lock_status.is_locked {
                return Ok(false);
            }

            if let Some(locked_until) = lock_status.locked_until {
                let now = chrono::Utc::now();
                if now > locked_until {
                    // Auto-unlock by setting is_locked = false
                    let unlock_sql = "UPDATE users SET is_locked = false, failed_login_attempts = 0 WHERE id = $1";
                    let unlock_stmt = Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        unlock_sql,
                        [user_id.into()],
                    );
                    self.db
                        .execute_raw(unlock_stmt)
                        .await
                        .map_err(|e| UserError::Internal(format!("Database error: {}", e)))?;
                    return Ok(false);
                }
            }

            Ok(lock_status.is_locked)
        } else {
            Err(UserError::NotFound(user_id))
        }
    }
}
