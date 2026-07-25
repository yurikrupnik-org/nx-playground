//! Data layer (sqlx). Every tenant-owned query is scoped by `org_id` (the primary
//! isolation control) and additionally runs inside a transaction that sets the
//! `app.org_id` GUC, so Postgres RLS applies as defense-in-depth when the API
//! connects as a non-superuser role.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::postgres::PgPoolOptions;
use sqlx::{FromRow, PgPool};
use utoipa::ToSchema;
use uuid::Uuid;

pub type Db = PgPool;

/// Connect and build a pooled Postgres handle.
pub async fn connect(url: &str) -> sqlx::Result<Db> {
    PgPoolOptions::new().max_connections(10).connect(url).await
}

// --- Identity / tenancy provisioning (non-RLS tables) ---------------------------

/// Find-or-create a user by IdP subject, refreshing the cached profile. Returns the id.
/// Does **not** touch `last_login_at` — that is bumped only on an actual login (see
/// [`touch_last_login`]), never on every authenticated request.
pub async fn upsert_user(db: &Db, subject: &str, email: &str, name: &str) -> sqlx::Result<Uuid> {
    let row: (Uuid,) = sqlx::query_as(
        "INSERT INTO users (subject, email, name) VALUES ($1, $2, $3)
         ON CONFLICT (subject) DO UPDATE SET email = EXCLUDED.email, name = EXCLUDED.name
         RETURNING id",
    )
    .bind(subject)
    .bind(email)
    .bind(name)
    .fetch_one(db)
    .await?;
    Ok(row.0)
}

/// Bump `last_login_at` on a real login (called from the OAuth callback only).
pub async fn touch_last_login(db: &Db, subject: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE users SET last_login_at = now() WHERE subject = $1")
        .bind(subject)
        .execute(db)
        .await?;
    Ok(())
}

/// Read-only lookup of a user's internal id by IdP subject.
pub async fn find_user_id(db: &Db, subject: &str) -> sqlx::Result<Option<Uuid>> {
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE subject = $1")
        .bind(subject)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.0))
}

/// Read-only lookup of an organization's internal id by external (IdP) id.
pub async fn find_org_id(db: &Db, external_org_id: &str) -> sqlx::Result<Option<Uuid>> {
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM organizations WHERE external_org_id = $1")
            .bind(external_org_id)
            .fetch_optional(db)
            .await?;
    Ok(row.map(|r| r.0))
}

/// Read-only lookup of a user's role within an org. RLS-scoped: sets `app.org_id`
/// so the `memberships` policy admits the row under a non-superuser role.
pub async fn find_membership_role(
    db: &Db,
    user_id: Uuid,
    org_id: Uuid,
) -> sqlx::Result<Option<String>> {
    let mut tx = db.begin().await?;
    set_org_guc(&mut tx, org_id).await?;
    let row: Option<(String,)> =
        sqlx::query_as("SELECT role::text FROM memberships WHERE user_id = $1 AND org_id = $2")
            .bind(user_id)
            .bind(org_id)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(row.map(|r| r.0))
}

/// Find-or-create an organization by its external (IdP) id. Returns the id.
pub async fn upsert_org(db: &Db, external_org_id: &str, name: &str) -> sqlx::Result<Uuid> {
    let row: (Uuid,) = sqlx::query_as(
        "INSERT INTO organizations (external_org_id, name) VALUES ($1, $2)
         ON CONFLICT (external_org_id) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .bind(external_org_id)
    .bind(name)
    .fetch_one(db)
    .await?;
    Ok(row.0)
}

/// Ensure a membership row exists for (user, org) with the given role.
/// RLS-scoped: runs with `app.org_id` set to `org_id`.
pub async fn ensure_membership(
    db: &Db,
    user_id: Uuid,
    org_id: Uuid,
    role: &str,
) -> sqlx::Result<()> {
    let mut tx = db.begin().await?;
    set_org_guc(&mut tx, org_id).await?;
    sqlx::query(
        "INSERT INTO memberships (user_id, org_id, role) VALUES ($1, $2, $3::org_role)
         ON CONFLICT (user_id, org_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(user_id)
    .bind(org_id)
    .bind(role)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

// --- Cloud assets (RLS table, tenant-scoped) ------------------------------------

/// A discovered cloud asset (the Phase 3 sample tenant-scoped resource).
#[derive(Debug, Clone, Serialize, FromRow, PartialEq, ToSchema)]
pub struct CloudAsset {
    pub id: Uuid,
    pub org_id: Uuid,
    pub provider: String,
    pub external_id: String,
    pub name: String,
    pub asset_type: String,
    pub region: String,
    pub status: String,
    pub monthly_cost: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a cloud asset.
pub struct NewAsset {
    pub provider: String,
    pub external_id: String,
    pub name: String,
    pub asset_type: String,
    pub region: String,
    pub status: String,
    pub monthly_cost: f64,
    pub metadata: serde_json::Value,
}

const ASSET_COLUMNS: &str = "id, org_id, provider::text AS provider, external_id, name, \
     asset_type, region, status::text AS status, monthly_cost::float8 AS monthly_cost, \
     created_at, updated_at";

/// List all assets for an organization (scoped by `org_id`).
pub async fn list_assets_for_org(db: &Db, org_id: Uuid) -> sqlx::Result<Vec<CloudAsset>> {
    let mut tx = db.begin().await?;
    set_org_guc(&mut tx, org_id).await?;
    let sql = format!(
        "SELECT {ASSET_COLUMNS} FROM cloud_assets WHERE org_id = $1 ORDER BY created_at DESC"
    );
    let assets = sqlx::query_as::<_, CloudAsset>(sqlx::AssertSqlSafe(sql))
        .bind(org_id)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(assets)
}

/// List the assets within an organization discovered by a specific user.
/// Tenant-scoped (`org_id` + RLS); `discovered_by` only narrows *within* that org,
/// so an arbitrary `user_id` can never surface another tenant's rows.
pub async fn list_assets_for_user(
    db: &Db,
    org_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<CloudAsset>> {
    let mut tx = db.begin().await?;
    set_org_guc(&mut tx, org_id).await?;
    let sql = format!(
        "SELECT {ASSET_COLUMNS} FROM cloud_assets \
         WHERE org_id = $1 AND discovered_by = $2 ORDER BY created_at DESC"
    );
    let assets = sqlx::query_as::<_, CloudAsset>(sqlx::AssertSqlSafe(sql))
        .bind(org_id)
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(assets)
}

/// Fetch a single asset by id **within** an organization. Returns `None` if the
/// asset does not exist or belongs to another tenant (no cross-tenant read).
pub async fn get_asset_for_org(
    db: &Db,
    org_id: Uuid,
    asset_id: Uuid,
) -> sqlx::Result<Option<CloudAsset>> {
    let mut tx = db.begin().await?;
    set_org_guc(&mut tx, org_id).await?;
    let sql = format!("SELECT {ASSET_COLUMNS} FROM cloud_assets WHERE org_id = $1 AND id = $2");
    let asset = sqlx::query_as::<_, CloudAsset>(sqlx::AssertSqlSafe(sql))
        .bind(org_id)
        .bind(asset_id)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(asset)
}

/// Insert an asset for an organization, returning the created row.
pub async fn create_asset(
    db: &Db,
    org_id: Uuid,
    discovered_by: Option<Uuid>,
    input: &NewAsset,
) -> sqlx::Result<CloudAsset> {
    let mut tx = db.begin().await?;
    set_org_guc(&mut tx, org_id).await?;
    let sql = format!(
        "INSERT INTO cloud_assets
           (org_id, discovered_by, provider, external_id, name, asset_type, region, status, monthly_cost, metadata)
         VALUES ($1, $2, $3::cloud_provider, $4, $5, $6, $7, $8::asset_status, $9::numeric, $10::jsonb)
         RETURNING {ASSET_COLUMNS}"
    );
    let asset = sqlx::query_as::<_, CloudAsset>(sqlx::AssertSqlSafe(sql))
        .bind(org_id)
        .bind(discovered_by)
        .bind(&input.provider)
        .bind(&input.external_id)
        .bind(&input.name)
        .bind(&input.asset_type)
        .bind(&input.region)
        .bind(&input.status)
        .bind(input.monthly_cost)
        .bind(input.metadata.to_string())
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(asset)
}

/// Set the per-transaction `app.org_id` GUC consumed by RLS policies.
async fn set_org_guc(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('app.org_id', $1, true)")
        .bind(org_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
