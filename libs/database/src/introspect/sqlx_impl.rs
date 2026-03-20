use sqlx::{PgPool, Row};

use super::{
    ExtensionInfo, FunctionInfo, IntrospectError, QueryPlan, SettingInfo, TableInfo,
    LIST_EXTENSIONS_SQL, LIST_FUNCTIONS_SQL, LIST_SETTINGS_SQL, LIST_TABLES_SQL,
};

/// PostgreSQL introspector backed by a raw sqlx `PgPool`.
#[derive(Clone)]
pub struct SqlxIntrospector {
    pool: PgPool,
}

impl SqlxIntrospector {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list_tables(&self) -> Result<Vec<TableInfo>, IntrospectError> {
        let rows = sqlx::query(LIST_TABLES_SQL)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows
            .iter()
            .map(|r| TableInfo {
                table_name: r.get("table_name"),
                table_type: r.get("table_type"),
                description: r.get("description"),
                estimated_rows: r.get("estimated_rows"),
            })
            .collect())
    }

    pub async fn list_extensions(&self) -> Result<Vec<ExtensionInfo>, IntrospectError> {
        let rows = sqlx::query(LIST_EXTENSIONS_SQL)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows
            .iter()
            .map(|r| ExtensionInfo {
                name: r.get("name"),
                version: r.get("version"),
                schema: r.get("schema"),
                description: r.get("description"),
            })
            .collect())
    }

    pub async fn list_settings(
        &self,
        category: Option<&str>,
    ) -> Result<Vec<SettingInfo>, IntrospectError> {
        let rows = match category {
            Some(cat) => {
                let sql = format!(
                    "{} WHERE category = $1",
                    LIST_SETTINGS_SQL.replace("ORDER BY", "AND 1=1 ORDER BY")
                );
                // Simpler: just filter in Rust to avoid mutating the SQL
                let all = sqlx::query(LIST_SETTINGS_SQL)
                    .fetch_all(&self.pool)
                    .await?;
                all.into_iter()
                    .filter(|r| {
                        let c: String = r.get("category");
                        c.to_lowercase().contains(&cat.to_lowercase())
                    })
                    .collect::<Vec<_>>()
            }
            None => sqlx::query(LIST_SETTINGS_SQL)
                .fetch_all(&self.pool)
                .await?,
        };

        Ok(rows
            .iter()
            .map(|r| SettingInfo {
                name: r.get("name"),
                value: r.get("value"),
                unit: r.get("unit"),
                category: r.get("category"),
                description: r.get("description"),
                context: r.get("context"),
            })
            .collect())
    }

    pub async fn get_query_plan(&self, query: &str) -> Result<QueryPlan, IntrospectError> {
        let explain_sql = format!("EXPLAIN (FORMAT JSON) {query}");
        let row = sqlx::query(&explain_sql)
            .fetch_one(&self.pool)
            .await?;
        let plan: serde_json::Value = row.get(0);
        Ok(QueryPlan { plan })
    }

    pub async fn list_functions(&self) -> Result<Vec<FunctionInfo>, IntrospectError> {
        let rows = sqlx::query(LIST_FUNCTIONS_SQL)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows
            .iter()
            .map(|r| FunctionInfo {
                name: r.get("name"),
                schema: r.get("schema"),
                arguments: r.get("arguments"),
                return_type: r.get("return_type"),
                language: r.get("language"),
            })
            .collect())
    }
}
