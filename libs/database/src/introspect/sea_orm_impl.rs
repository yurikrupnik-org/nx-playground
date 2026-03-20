use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

use super::{
    ExtensionInfo, FunctionInfo, IntrospectError, QueryPlan, SettingInfo, TableInfo,
    LIST_EXTENSIONS_SQL, LIST_FUNCTIONS_SQL, LIST_SETTINGS_SQL, LIST_TABLES_SQL,
};

/// PostgreSQL introspector backed by a SeaORM `DatabaseConnection`.
#[derive(Clone)]
pub struct SeaOrmIntrospector {
    db: DatabaseConnection,
}

impl SeaOrmIntrospector {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    fn stmt(sql: &str) -> Statement {
        Statement::from_string(DatabaseBackend::Postgres, sql)
    }

    pub async fn list_tables(&self) -> Result<Vec<TableInfo>, IntrospectError> {
        let rows = self.db.query_all(Self::stmt(LIST_TABLES_SQL)).await?;

        rows.iter()
            .map(|r| {
                Ok(TableInfo {
                    table_name: r.try_get("", "table_name")?,
                    table_type: r.try_get("", "table_type")?,
                    description: r.try_get("", "description")?,
                    estimated_rows: r.try_get("", "estimated_rows")?,
                })
            })
            .collect()
    }

    pub async fn list_extensions(&self) -> Result<Vec<ExtensionInfo>, IntrospectError> {
        let rows = self.db.query_all(Self::stmt(LIST_EXTENSIONS_SQL)).await?;

        rows.iter()
            .map(|r| {
                Ok(ExtensionInfo {
                    name: r.try_get("", "name")?,
                    version: r.try_get("", "version")?,
                    schema: r.try_get("", "schema")?,
                    description: r.try_get("", "description")?,
                })
            })
            .collect()
    }

    pub async fn list_settings(
        &self,
        category: Option<&str>,
    ) -> Result<Vec<SettingInfo>, IntrospectError> {
        let rows = self.db.query_all(Self::stmt(LIST_SETTINGS_SQL)).await?;

        let mut settings: Vec<SettingInfo> = rows
            .iter()
            .map(|r| {
                Ok(SettingInfo {
                    name: r.try_get("", "name")?,
                    value: r.try_get("", "value")?,
                    unit: r.try_get("", "unit")?,
                    category: r.try_get("", "category")?,
                    description: r.try_get("", "description")?,
                    context: r.try_get("", "context")?,
                })
            })
            .collect::<Result<Vec<_>, IntrospectError>>()?;

        if let Some(cat) = category {
            let cat_lower = cat.to_lowercase();
            settings.retain(|s| s.category.to_lowercase().contains(&cat_lower));
        }

        Ok(settings)
    }

    pub async fn get_query_plan(&self, query: &str) -> Result<QueryPlan, IntrospectError> {
        let explain_sql = format!("EXPLAIN (FORMAT JSON) {query}");
        let row = self
            .db
            .query_one(Self::stmt(&explain_sql))
            .await?
            .ok_or_else(|| IntrospectError::Other("empty EXPLAIN result".into()))?;

        let plan: serde_json::Value = row.try_get("", "QUERY PLAN")?;
        Ok(QueryPlan { plan })
    }

    pub async fn list_functions(&self) -> Result<Vec<FunctionInfo>, IntrospectError> {
        let rows = self.db.query_all(Self::stmt(LIST_FUNCTIONS_SQL)).await?;

        rows.iter()
            .map(|r| {
                Ok(FunctionInfo {
                    name: r.try_get("", "name")?,
                    schema: r.try_get("", "schema")?,
                    arguments: r.try_get("", "arguments")?,
                    return_type: r.try_get("", "return_type")?,
                    language: r.try_get("", "language")?,
                })
            })
            .collect()
    }
}
