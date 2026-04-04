//! PostgreSQL introspection queries.
//!
//! Supports two backends controlled by Cargo feature flags:
//! - `introspect-sqlx`: Raw sqlx queries against `PgPool`
//! - `introspect-sea-orm`: Raw SQL via SeaORM's `ConnectionTrait`
//!
//! When both features are enabled, use the [`Introspector`] enum for runtime dispatch
//! (e.g. driven by Flagsmith).

#[cfg(feature = "introspect-sea-orm")]
mod sea_orm_impl;
#[cfg(feature = "introspect-sqlx")]
mod sqlx_impl;

#[cfg(feature = "introspect-sea-orm")]
pub use sea_orm_impl::SeaOrmIntrospector;
#[cfg(feature = "introspect-sqlx")]
pub use sqlx_impl::SqlxIntrospector;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub table_name: String,
    pub table_type: String,
    pub description: Option<String>,
    pub estimated_rows: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInfo {
    pub name: String,
    pub version: String,
    pub schema: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingInfo {
    pub name: String,
    pub value: Option<String>,
    pub unit: Option<String>,
    pub category: String,
    pub description: String,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    pub schema: String,
    pub arguments: String,
    pub return_type: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryPlan {
    pub plan: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum IntrospectError {
    #[cfg(feature = "introspect-sea-orm")]
    #[error("SeaORM error: {0}")]
    SeaOrm(#[from] sea_orm::DbErr),

    #[cfg(feature = "introspect-sqlx")]
    #[error("Sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// SQL constants (shared by both backends)
// ---------------------------------------------------------------------------

pub(crate) const LIST_TABLES_SQL: &str = r#"
SELECT
    t.table_name,
    t.table_type,
    pg_catalog.obj_description(c.oid) AS description,
    COALESCE(c.reltuples::bigint, 0) AS estimated_rows
FROM information_schema.tables t
LEFT JOIN pg_catalog.pg_class c
    ON c.relname = t.table_name
    AND c.relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = t.table_schema)
WHERE t.table_schema = 'public'
ORDER BY t.table_name
"#;

pub(crate) const LIST_EXTENSIONS_SQL: &str = r#"
SELECT
    e.extname AS name,
    e.extversion AS version,
    n.nspname AS schema,
    c.description
FROM pg_extension e
JOIN pg_namespace n ON e.extnamespace = n.oid
LEFT JOIN pg_description c ON c.objoid = e.oid AND c.classoid = 'pg_extension'::regclass
ORDER BY e.extname
"#;

pub(crate) const LIST_SETTINGS_SQL: &str = r#"
SELECT
    name,
    setting AS value,
    unit,
    category,
    short_desc AS description,
    context
FROM pg_settings
ORDER BY category, name
"#;

pub(crate) const LIST_FUNCTIONS_SQL: &str = r#"
SELECT
    p.proname AS name,
    n.nspname AS schema,
    pg_catalog.pg_get_function_arguments(p.oid) AS arguments,
    pg_catalog.pg_get_function_result(p.oid) AS return_type,
    l.lanname AS language
FROM pg_proc p
JOIN pg_namespace n ON p.pronamespace = n.oid
JOIN pg_language l ON p.prolang = l.oid
WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
ORDER BY n.nspname, p.proname
"#;

// ---------------------------------------------------------------------------
// Enum dispatch (available when at least one backend is compiled)
// ---------------------------------------------------------------------------

/// Runtime-switchable introspector.
///
/// Build with both `introspect-sqlx` and `introspect-sea-orm` features, then
/// pick a variant at runtime (e.g. based on a Flagsmith flag).
pub enum Introspector {
    #[cfg(feature = "introspect-sqlx")]
    Sqlx(SqlxIntrospector),
    #[cfg(feature = "introspect-sea-orm")]
    SeaOrm(SeaOrmIntrospector),
}

impl Introspector {
    pub async fn list_tables(&self) -> Result<Vec<TableInfo>, IntrospectError> {
        match self {
            #[cfg(feature = "introspect-sqlx")]
            Self::Sqlx(i) => i.list_tables().await,
            #[cfg(feature = "introspect-sea-orm")]
            Self::SeaOrm(i) => i.list_tables().await,
        }
    }

    pub async fn list_extensions(&self) -> Result<Vec<ExtensionInfo>, IntrospectError> {
        match self {
            #[cfg(feature = "introspect-sqlx")]
            Self::Sqlx(i) => i.list_extensions().await,
            #[cfg(feature = "introspect-sea-orm")]
            Self::SeaOrm(i) => i.list_extensions().await,
        }
    }

    pub async fn list_settings(
        &self,
        category: Option<&str>,
    ) -> Result<Vec<SettingInfo>, IntrospectError> {
        match self {
            #[cfg(feature = "introspect-sqlx")]
            Self::Sqlx(i) => i.list_settings(category).await,
            #[cfg(feature = "introspect-sea-orm")]
            Self::SeaOrm(i) => i.list_settings(category).await,
        }
    }

    pub async fn get_query_plan(&self, query: &str) -> Result<QueryPlan, IntrospectError> {
        match self {
            #[cfg(feature = "introspect-sqlx")]
            Self::Sqlx(i) => i.get_query_plan(query).await,
            #[cfg(feature = "introspect-sea-orm")]
            Self::SeaOrm(i) => i.get_query_plan(query).await,
        }
    }

    pub async fn list_functions(&self) -> Result<Vec<FunctionInfo>, IntrospectError> {
        match self {
            #[cfg(feature = "introspect-sqlx")]
            Self::Sqlx(i) => i.list_functions().await,
            #[cfg(feature = "introspect-sea-orm")]
            Self::SeaOrm(i) => i.list_functions().await,
        }
    }

    /// Which backend this introspector is using (useful for logging / responses).
    pub fn backend_name(&self) -> &'static str {
        match self {
            #[cfg(feature = "introspect-sqlx")]
            Self::Sqlx(_) => "sqlx",
            #[cfg(feature = "introspect-sea-orm")]
            Self::SeaOrm(_) => "sea-orm",
        }
    }
}
