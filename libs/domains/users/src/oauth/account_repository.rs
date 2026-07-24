use super::account::{CreateOAuthAccountParams, OAuthAccount};
use crate::error::{UserError, UserResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DbBackend, FromQueryResult, Statement};
use uuid::Uuid;

/// Repository for OAuth account operations
#[async_trait]
pub trait OAuthAccountRepository: Send + Sync + Clone {
    async fn create(&self, params: CreateOAuthAccountParams<'_>) -> UserResult<OAuthAccount>;
    async fn find_by_provider_and_user_id(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> UserResult<Option<OAuthAccount>>;
    async fn find_by_user_id(&self, user_id: Uuid) -> UserResult<Vec<OAuthAccount>>;
    async fn find_by_user_id_and_provider(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> UserResult<Option<OAuthAccount>>;
    async fn update_tokens(
        &self,
        id: Uuid,
        access_token: Option<&str>,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> UserResult<()>;
    async fn delete_by_user_and_provider(&self, user_id: Uuid, provider: &str) -> UserResult<bool>;
}

/// PostgreSQL implementation of OAuthAccountRepository
#[derive(Clone)]
pub struct PostgresOAuthAccountRepository {
    db: sea_orm::DatabaseConnection,
}

impl PostgresOAuthAccountRepository {
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self { db }
    }
}

#[derive(Debug, FromQueryResult)]
struct OAuthAccountRow {
    id: Uuid,
    user_id: Uuid,
    provider: String,
    provider_user_id: String,
    provider_username: Option<String>,
    email: Option<String>,
    display_name: Option<String>,
    avatar_url: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_expires_at: Option<DateTime<Utc>>,
    scopes: Option<Vec<String>>,
    raw_user_data: Option<serde_json::Value>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<OAuthAccountRow> for OAuthAccount {
    fn from(row: OAuthAccountRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            provider: row.provider,
            provider_user_id: row.provider_user_id,
            provider_username: row.provider_username,
            email: row.email,
            display_name: row.display_name,
            avatar_url: row.avatar_url,
            access_token: row.access_token,
            refresh_token: row.refresh_token,
            token_expires_at: row.token_expires_at,
            scopes: row.scopes,
            raw_user_data: row.raw_user_data,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[async_trait]
impl OAuthAccountRepository for PostgresOAuthAccountRepository {
    async fn create(&self, params: CreateOAuthAccountParams<'_>) -> UserResult<OAuthAccount> {
        let now = Utc::now();
        let id = Uuid::now_v7();

        let sql = r#"
            INSERT INTO oauth_accounts (
                id, user_id, provider, provider_user_id, provider_username,
                email, display_name, avatar_url, access_token, refresh_token,
                token_expires_at, scopes, raw_user_data, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            RETURNING *
        "#;

        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [
                id.into(),
                params.user_id.into(),
                params.provider.into(),
                params.provider_user_id.into(),
                params.provider_username.into(),
                params.email.into(),
                params.display_name.into(),
                params.avatar_url.into(),
                params.access_token.into(),
                params.refresh_token.into(),
                params.token_expires_at.into(),
                params.scopes.into(),
                params.raw_user_data.into(),
                now.into(),
                now.into(),
            ],
        );

        let row = OAuthAccountRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?
            .ok_or_else(|| UserError::Internal("Failed to create OAuth account".to_string()))?;

        Ok(row.into())
    }

    async fn find_by_provider_and_user_id(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> UserResult<Option<OAuthAccount>> {
        let sql = "SELECT * FROM oauth_accounts WHERE provider = $1 AND provider_user_id = $2";

        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [provider.into(), provider_user_id.into()],
        );

        let row = OAuthAccountRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(row.map(Into::into))
    }

    async fn find_by_user_id(&self, user_id: Uuid) -> UserResult<Vec<OAuthAccount>> {
        let sql = "SELECT * FROM oauth_accounts WHERE user_id = $1 ORDER BY created_at DESC";

        let stmt = Statement::from_sql_and_values(DbBackend::Postgres, sql, [user_id.into()]);

        let rows = OAuthAccountRow::find_by_statement(stmt)
            .all(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn find_by_user_id_and_provider(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> UserResult<Option<OAuthAccount>> {
        let sql = "SELECT * FROM oauth_accounts WHERE user_id = $1 AND provider = $2";

        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [user_id.into(), provider.into()],
        );

        let row = OAuthAccountRow::find_by_statement(stmt)
            .one(&self.db)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(row.map(Into::into))
    }

    async fn update_tokens(
        &self,
        id: Uuid,
        access_token: Option<&str>,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> UserResult<()> {
        let now = Utc::now();
        let sql = r#"
            UPDATE oauth_accounts
            SET access_token = COALESCE($2, access_token),
                refresh_token = COALESCE($3, refresh_token),
                token_expires_at = COALESCE($4, token_expires_at),
                updated_at = $5
            WHERE id = $1
        "#;

        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [
                id.into(),
                access_token.into(),
                refresh_token.into(),
                token_expires_at.into(),
                now.into(),
            ],
        );

        self.db
            .execute_raw(stmt)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(())
    }

    async fn delete_by_user_and_provider(&self, user_id: Uuid, provider: &str) -> UserResult<bool> {
        let sql = "DELETE FROM oauth_accounts WHERE user_id = $1 AND provider = $2";

        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [user_id.into(), provider.into()],
        );

        let result = self
            .db
            .execute_raw(stmt)
            .await
            .map_err(|e| UserError::Internal(format!("Database error: {e}")))?;

        Ok(result.rows_affected() > 0)
    }
}
