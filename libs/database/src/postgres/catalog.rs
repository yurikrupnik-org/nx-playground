//! Live Postgres catalog introspection and row sampling (feature `catalog`).
//!
//! Generic infrastructure, not domain data: it describes *any* Postgres
//! database reachable with the configured credentials — every connectable,
//! non-template database on the server, its relations (tables, views,
//! materialized views, partitioned and foreign tables), columns, enum labels
//! and foreign keys — and pages through a relation's rows as JSON.
//!
//! Everything is read from `pg_catalog`, never `information_schema`, so views,
//! materialized views and enum types are covered and privileges do not hide
//! objects. SQL identifiers are never taken from the caller: a requested
//! schema/table is first looked up in the catalog and only the names the
//! catalog returns are quoted into the row query. `limit`/`offset` are bound
//! parameters.

use std::time::Duration;

use sea_orm::sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, DbErr, SqlxPostgresConnector, Statement, Value,
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

/// Default page size for [`Catalog::sample_rows`].
pub const DEFAULT_ROW_LIMIT: i64 = 50;
/// Largest page [`Catalog::sample_rows`] returns; larger requests are clamped.
pub const MAX_ROW_LIMIT: i64 = 500;

/// How long to wait for a sibling database connection before reporting it as
/// unreachable.
const SIBLING_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Catalog failure.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// The requested database, schema or relation does not exist (or is not
    /// browsable: templates, `postgres`, system schemas).
    #[error("{0} not found")]
    NotFound(String),
    /// A sibling database exists but could not be connected to.
    #[error("cannot connect to database {database}: {reason}")]
    Connect { database: String, reason: String },
    /// A catalog or row query failed.
    #[error("query failed: {0}")]
    Query(#[from] DbErr),
    /// The catalog query returned JSON that does not match the DTOs.
    #[error("unexpected catalog shape: {0}")]
    Decode(#[from] serde_json::Error),
}

pub type CatalogResult<T> = Result<T, CatalogError>;

/// One database on the server.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CatalogDatabase {
    pub name: String,
    /// `true` for the database the service's own connection points at.
    pub current: bool,
    /// Why this database could not be connected to or introspected; `tables`
    /// is then empty. One broken database never fails the whole listing.
    pub error: Option<String>,
    pub tables: Vec<CatalogTable>,
}

/// Kind of a browsable relation (`pg_class.relkind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Table,
    View,
    MaterializedView,
    PartitionedTable,
    ForeignTable,
}

/// A table-like relation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CatalogTable {
    pub schema: String,
    pub name: String,
    pub kind: RelationKind,
    pub comment: Option<String>,
    /// Planner estimate (`pg_class.reltuples`); `null` when never analyzed
    /// or when the relation stores no rows itself (views).
    pub row_estimate: Option<i64>,
    pub columns: Vec<CatalogColumn>,
    pub foreign_keys: Vec<CatalogForeignKey>,
}

/// A column of a [`CatalogTable`].
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CatalogColumn {
    pub name: String,
    /// 1-based ordinal (`pg_attribute.attnum`).
    pub position: i32,
    /// `format_type(atttypid, atttypmod)`, e.g. `character varying(255)`.
    pub data_type: String,
    pub nullable: bool,
    /// Default (or generation) expression as SQL text.
    pub default: Option<String>,
    pub is_primary_key: bool,
    pub comment: Option<String>,
    /// Labels in sort order when the column's type (or its array element /
    /// domain base type) is an enum.
    pub enum_values: Option<Vec<String>>,
}

/// A foreign key declared on a [`CatalogTable`].
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CatalogForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_schema: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
}

/// One page of a relation's rows.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RowsPage {
    /// Column names in position order.
    pub columns: Vec<String>,
    /// `row_to_json` objects, keys in column order. Kept as raw JSON so the
    /// server's rendering (key order, numeric precision) reaches the client
    /// untouched.
    #[schema(value_type = Vec<Object>)]
    pub rows: Box<RawValue>,
    /// Exact `count(*)` of the relation.
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Catalog over one Postgres server, anchored at the service's own connection.
///
/// `database_url` is the URL `current` was opened with; sibling databases are
/// reached with the same credentials, host, port and parameters, with only the
/// database name swapped.
pub struct Catalog<'a> {
    current: &'a DatabaseConnection,
    database_url: &'a str,
}

/// Connection to the database being inspected: the service's shared pool, or a
/// one-connection pool opened for this request and closed after it.
enum Target<'a> {
    Current(&'a DatabaseConnection),
    Sibling(DatabaseConnection),
}

impl Target<'_> {
    fn conn(&self) -> &DatabaseConnection {
        match self {
            Target::Current(conn) => conn,
            Target::Sibling(conn) => conn,
        }
    }

    async fn close(self) {
        if let Target::Sibling(conn) = self
            && let Err(e) = conn.close().await
        {
            tracing::debug!(error = %e, "closing catalog sibling connection");
        }
    }
}

/// A browsable database as listed by `pg_database`.
struct DatabaseEntry {
    name: String,
    current: bool,
}

impl<'a> Catalog<'a> {
    pub fn new(current: &'a DatabaseConnection, database_url: &'a str) -> Self {
        Self {
            current,
            database_url,
        }
    }

    /// Every connectable, non-template database except `postgres`, sorted by
    /// name, each with its introspected relations (or the error that
    /// prevented introspection).
    pub async fn databases(&self) -> CatalogResult<Vec<CatalogDatabase>> {
        let mut out = Vec::new();
        for entry in self.database_entries().await? {
            let tables = match self.open(&entry).await {
                Ok(target) => {
                    let tables = introspect(target.conn()).await;
                    target.close().await;
                    tables
                }
                Err(e) => Err(e),
            };
            let (tables, error) = match tables {
                Ok(tables) => (tables, None),
                Err(e) => {
                    tracing::warn!(database = %entry.name, error = %e, "catalog introspection failed");
                    (Vec::new(), Some(e.to_string()))
                }
            };
            out.push(CatalogDatabase {
                name: entry.name,
                current: entry.current,
                error,
                tables,
            });
        }
        Ok(out)
    }

    /// One page of `schema.table` in `database`. `limit` is clamped to
    /// `1..=MAX_ROW_LIMIT` and `offset` to `>= 0`; rows are ordered by the
    /// primary key when the relation has one.
    pub async fn sample_rows(
        &self,
        database: &str,
        schema: &str,
        table: &str,
        limit: i64,
        offset: i64,
    ) -> CatalogResult<RowsPage> {
        let limit = limit.clamp(1, MAX_ROW_LIMIT);
        let offset = offset.max(0);

        let entry = self
            .database_entries()
            .await?
            .into_iter()
            .find(|entry| entry.name == database)
            .ok_or_else(|| CatalogError::NotFound(format!("database {database}")))?;

        let target = self.open(&entry).await?;
        let page = rows_page(target.conn(), schema, table, limit, offset).await;
        target.close().await;
        page
    }

    async fn database_entries(&self) -> CatalogResult<Vec<DatabaseEntry>> {
        let rows = self
            .current
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT datname, datname = current_database() AS current \
                 FROM pg_catalog.pg_database \
                 WHERE datallowconn AND NOT datistemplate AND datname <> 'postgres' \
                 ORDER BY datname",
            ))
            .await?;
        rows.iter()
            .map(|row| {
                Ok(DatabaseEntry {
                    name: row.try_get("", "datname")?,
                    current: row.try_get("", "current")?,
                })
            })
            .collect()
    }

    async fn open(&self, entry: &DatabaseEntry) -> CatalogResult<Target<'a>> {
        if entry.current {
            return Ok(Target::Current(self.current));
        }
        let connect_err = |reason: String| CatalogError::Connect {
            database: entry.name.clone(),
            reason,
        };
        let options = sibling_options(self.database_url, &entry.name)
            .map_err(|e| connect_err(e.to_string()))?;
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(SIBLING_CONNECT_TIMEOUT)
            .connect_with(options)
            .await
            .map_err(|e| connect_err(e.to_string()))?;
        Ok(Target::Sibling(
            SqlxPostgresConnector::from_sqlx_postgres_pool(pool),
        ))
    }
}

/// Connect options for database `name` on the server `database_url` points at:
/// same user, password, host, port and parameters.
fn sibling_options(
    database_url: &str,
    name: &str,
) -> Result<PgConnectOptions, sea_orm::sqlx::Error> {
    Ok(database_url.parse::<PgConnectOptions>()?.database(name))
}

/// Double-quote an identifier, doubling embedded quotes.
fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Relations of one database as JSON built by Postgres itself, one row per
/// relation, already in contract shape.
const INTROSPECT_SQL: &str = r"
SELECT json_build_object(
  'schema', n.nspname,
  'name', c.relname,
  'kind', CASE c.relkind
            WHEN 'r' THEN 'table'
            WHEN 'v' THEN 'view'
            WHEN 'm' THEN 'materialized_view'
            WHEN 'p' THEN 'partitioned_table'
            WHEN 'f' THEN 'foreign_table'
          END,
  'comment', pg_catalog.obj_description(c.oid, 'pg_class'),
  'row_estimate', CASE WHEN c.relkind = 'v' OR c.reltuples < 0 THEN NULL
                       ELSE c.reltuples::bigint END,
  'columns', COALESCE((
    SELECT json_agg(json_build_object(
      'name', a.attname,
      'position', a.attnum,
      'data_type', pg_catalog.format_type(a.atttypid, a.atttypmod),
      'nullable', NOT a.attnotnull,
      'default', pg_catalog.pg_get_expr(d.adbin, d.adrelid),
      'is_primary_key', COALESCE(a.attnum = ANY (pk.conkey), false),
      'comment', pg_catalog.col_description(a.attrelid, a.attnum),
      'enum_values', (
        SELECT json_agg(e.enumlabel ORDER BY e.enumsortorder)
        FROM pg_catalog.pg_enum e
        WHERE e.enumtypid = CASE
          WHEN t.typtype = 'd' THEN t.typbasetype
          WHEN t.typcategory = 'A' THEN t.typelem
          ELSE t.oid
        END
      )
    ) ORDER BY a.attnum)
    FROM pg_catalog.pg_attribute a
    JOIN pg_catalog.pg_type t ON t.oid = a.atttypid
    LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
    WHERE a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
  ), '[]'::json),
  'foreign_keys', COALESCE((
    SELECT json_agg(json_build_object(
      'name', fk.conname,
      'columns', (
        SELECT json_agg(att.attname ORDER BY k.ord)
        FROM unnest(fk.conkey) WITH ORDINALITY AS k(attnum, ord)
        JOIN pg_catalog.pg_attribute att ON att.attrelid = fk.conrelid AND att.attnum = k.attnum
      ),
      'ref_schema', rn.nspname,
      'ref_table', rc.relname,
      'ref_columns', (
        SELECT json_agg(att.attname ORDER BY k.ord)
        FROM unnest(fk.confkey) WITH ORDINALITY AS k(attnum, ord)
        JOIN pg_catalog.pg_attribute att ON att.attrelid = fk.confrelid AND att.attnum = k.attnum
      )
    ) ORDER BY fk.conname)
    FROM pg_catalog.pg_constraint fk
    JOIN pg_catalog.pg_class rc ON rc.oid = fk.confrelid
    JOIN pg_catalog.pg_namespace rn ON rn.oid = rc.relnamespace
    WHERE fk.conrelid = c.oid AND fk.contype = 'f'
  ), '[]'::json)
)::text AS doc
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_catalog.pg_constraint pk ON pk.conrelid = c.oid AND pk.contype = 'p'
WHERE c.relkind IN ('r', 'v', 'm', 'p', 'f')
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND n.nspname !~ '^pg_(toast|temp_)'
ORDER BY n.nspname, c.relname
";

/// Every browsable relation of the database `conn` is connected to.
pub async fn introspect(conn: &DatabaseConnection) -> CatalogResult<Vec<CatalogTable>> {
    conn.query_all_raw(Statement::from_string(DbBackend::Postgres, INTROSPECT_SQL))
        .await?
        .iter()
        .map(|row| {
            let doc: String = row.try_get("", "doc")?;
            Ok(serde_json::from_str(&doc)?)
        })
        .collect()
}

/// Looks up `schema.table` among browsable relations; returns the catalog's
/// own spelling of both names, the column names in position order, and the
/// primary-key column names in key order.
const RELATION_SQL: &str = r"
SELECT n.nspname AS schema_name,
       c.relname AS table_name,
       (SELECT COALESCE(json_agg(a.attname ORDER BY a.attnum), '[]'::json)
        FROM pg_catalog.pg_attribute a
        WHERE a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped)::text AS columns,
       (SELECT COALESCE(json_agg(a.attname ORDER BY k.ord), '[]'::json)
        FROM pg_catalog.pg_constraint pk
        CROSS JOIN unnest(pk.conkey) WITH ORDINALITY AS k(attnum, ord)
        JOIN pg_catalog.pg_attribute a ON a.attrelid = pk.conrelid AND a.attnum = k.attnum
        WHERE pk.conrelid = c.oid AND pk.contype = 'p')::text AS primary_key
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1 AND c.relname = $2
  AND c.relkind IN ('r', 'v', 'm', 'p', 'f')
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND n.nspname !~ '^pg_(toast|temp_)'
";

async fn rows_page(
    conn: &DatabaseConnection,
    schema: &str,
    table: &str,
    limit: i64,
    offset: i64,
) -> CatalogResult<RowsPage> {
    let relation = conn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            RELATION_SQL,
            [Value::from(schema), Value::from(table)],
        ))
        .await?
        .ok_or_else(|| CatalogError::NotFound(format!("relation {schema}.{table}")))?;

    // Identifiers below come from the catalog row, never from the request.
    let schema_name: String = relation.try_get("", "schema_name")?;
    let table_name: String = relation.try_get("", "table_name")?;
    let columns: Vec<String> = serde_json::from_str(&relation.try_get::<String>("", "columns")?)?;
    let primary_key: Vec<String> =
        serde_json::from_str(&relation.try_get::<String>("", "primary_key")?)?;

    let qualified = format!("{}.{}", quote_ident(&schema_name), quote_ident(&table_name));
    let order_by = if primary_key.is_empty() {
        String::new()
    } else {
        let keys: Vec<String> = primary_key.iter().map(|k| quote_ident(k)).collect();
        format!(" ORDER BY {}", keys.join(", "))
    };

    let total: i64 = conn
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            format!("SELECT count(*) AS total FROM {qualified}"),
        ))
        .await?
        .ok_or_else(|| DbErr::RecordNotFound("count(*) returned no row".into()))?
        .try_get("", "total")?;

    let rows: String = conn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!(
                "SELECT COALESCE(json_agg(row_to_json(t)), '[]'::json)::text AS rows \
                 FROM (SELECT * FROM {qualified}{order_by} LIMIT $1 OFFSET $2) t"
            ),
            [Value::from(limit), Value::from(offset)],
        ))
        .await?
        .ok_or_else(|| DbErr::RecordNotFound("row page returned no row".into()))?
        .try_get("", "rows")?;

    Ok(RowsPage {
        columns,
        rows: RawValue::from_string(rows)?,
        total,
        limit,
        offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_ident_doubles_embedded_quotes() {
        assert_eq!(quote_ident("users"), r#""users""#);
        assert_eq!(quote_ident(r#"we"ird"#), r#""we""ird""#);
        assert_eq!(
            quote_ident(r#"x"; DROP TABLE t; --"#),
            r#""x""; DROP TABLE t; --""#
        );
    }

    #[test]
    fn sibling_options_swap_only_the_database() {
        let url =
            "postgres://myuser:p%40ss@db.local:6543/zerg?sslmode=disable&application_name=zerg";
        let options = sibling_options(url, "tasks").expect("valid url");
        assert_eq!(options.get_database(), Some("tasks"));
        assert_eq!(options.get_host(), "db.local");
        assert_eq!(options.get_port(), 6543);
        assert_eq!(options.get_username(), "myuser");
        assert_eq!(options.get_application_name(), Some("zerg"));
    }
}
