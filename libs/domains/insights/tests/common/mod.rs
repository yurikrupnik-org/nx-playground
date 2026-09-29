//! Shared fixtures: a Postgres container with the `insights` migrations.
//! Each test binary uses a subset of these helpers.
#![allow(dead_code)]

use domain_insights::Store;
use test_utils::TestDatabase;

pub async fn insights_db() -> (TestDatabase, Store) {
    let db = TestDatabase::with_migrations_dir("manifests/db/insights/migrations").await;
    let store = Store::connect(&db.connection_string)
        .await
        .expect("connect store");
    (db, store)
}

pub async fn count(store: &Store, table: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(store.pool())
        .await
        .expect("count");
    row.0
}
