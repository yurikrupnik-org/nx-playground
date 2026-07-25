use super::{User, UserError, UserFilter, UserRepository, UserResult};
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
    subject: Option<String>,
    roles: Vec<String>, // PostgreSQL text array
    email_verified: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    avatar_url: Option<String>,
    last_login_at: Option<chrono::DateTime<chrono::Utc>>,
    is_active: bool,
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
            subject: row.subject,
            roles,
            email_verified: row.email_verified,
            created_at: row.created_at,
            updated_at: row.updated_at,
            avatar_url: row.avatar_url,
            last_login_at: row.last_login_at,
            is_active: row.is_active,
        }
    }
}

#[async_trait]
impl UserRepository for PostgresUserRepository {
    async fn create(&self, user: User) -> UserResult<User> {
        let sql = r#"
            INSERT INTO users (id, email, name, subject, roles, email_verified, created_at, updated_at, last_login_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
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
                user.subject.clone().into(),
                roles_array.into(),
                user.email_verified.into(),
                user.created_at.into(),
                user.updated_at.into(),
                user.last_login_at.into(),
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
                    UserError::Internal(format!("Database error: {e}"))
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
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(row.map(|r| r.into()))
    }

    async fn get_by_email(&self, email: &str) -> UserResult<Option<User>> {
        let sql = "SELECT * FROM users WHERE email = $1";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [email.into()]);

        let row = UserRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(row.map(|r| r.into()))
    }

    async fn get_by_subject(&self, subject: &str) -> UserResult<Option<User>> {
        let sql = "SELECT * FROM users WHERE subject = $1";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [subject.into()]);

        let row = UserRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(row.map(|r| r.into()))
    }

    async fn set_subject(&self, user_id: Uuid, subject: &str) -> UserResult<()> {
        let sql = "UPDATE users SET subject = $2, updated_at = NOW() WHERE id = $1";

        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [user_id.into(), subject.into()],
        );

        let result = self
            .db
            .execute_raw(stmt)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(UserError::NotFound(user_id));
        }
        Ok(())
    }

    async fn touch_last_login(&self, user_id: Uuid) -> UserResult<()> {
        let sql = "UPDATE users SET last_login_at = NOW(), updated_at = NOW() WHERE id = $1";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [user_id.into()]);

        let result = self
            .db
            .execute_raw(stmt)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(UserError::NotFound(user_id));
        }
        Ok(())
    }

    async fn list(&self, _filter: UserFilter) -> UserResult<Vec<User>> {
        let sql = "SELECT * FROM users ORDER BY created_at DESC";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, []);

        let rows = UserRow::find_by_statement(stmt)
            .all(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn update(&self, user: User) -> UserResult<User> {
        let sql = r#"
            UPDATE users
            SET email = $2, name = $3, subject = $4, roles = $5,
                email_verified = $6, updated_at = $7, avatar_url = $8, last_login_at = $9,
                is_active = $10
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
                user.subject.clone().into(),
                roles_array.into(),
                user.email_verified.into(),
                user.updated_at.into(),
                user.avatar_url.clone().into(),
                user.last_login_at.into(),
                user.is_active.into(),
            ],
        );

        let row = UserRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        row.map(|r| r.into()).ok_or(UserError::NotFound(user.id))
    }

    async fn delete(&self, id: Uuid) -> UserResult<bool> {
        let sql = "DELETE FROM users WHERE id = $1";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [id.into()]);

        let result = self
            .db
            .execute_raw(stmt)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(result.rows_affected() > 0)
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
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

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
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(result.map(|r| r.count as usize).unwrap_or(0))
    }
}
