//! Performance comparison tests: PostgreSQL vs MongoDB vs gRPC
//!
//! These tests spin up real database containers and measure CRUD performance
//! across different backends and access patterns.
//!
//! Run individual backends:
//!   cargo test -p domain_tasks --test perf_test -- --nocapture              # PostgreSQL only
//!   cargo test -p domain_tasks --features mongo --test perf_test -- --nocapture  # All (PG + Mongo)
//!
//! Run specific tests:
//!   cargo test -p domain_tasks --test perf_test perf_postgres -- --nocapture
//!   cargo test -p domain_tasks --features mongo --test perf_test perf_mongodb -- --nocapture
//!   cargo test -p domain_tasks --features mongo --test perf_test perf_postgres_vs_mongodb -- --nocapture

use domain_tasks::{CreateTask, PgTaskRepository, TaskFilter, TaskRepository};
use std::time::Instant;
use test_utils::TestDatabase;

const BATCH_SIZE: usize = 100;
const LIST_ITERATIONS: usize = 50;

fn create_sample_task(i: usize) -> CreateTask {
    CreateTask {
        title: format!("Perf Test Task {i}"),
        description: format!("Description for performance test task number {i}"),
        project_id: None,
        priority: domain_tasks::TaskPriority::Medium,
        status: domain_tasks::TaskStatus::Todo,
        due_date: None,
    }
}

/// Measures create, get_by_id, list, update, and delete operations
async fn benchmark_repository<R: TaskRepository>(name: &str, repo: &R) -> BenchmarkResult {
    // --- CREATE ---
    let start = Instant::now();
    let mut task_ids = Vec::with_capacity(BATCH_SIZE);
    for i in 0..BATCH_SIZE {
        let task = repo.create(create_sample_task(i)).await.unwrap();
        task_ids.push(task.id);
    }
    let create_duration = start.elapsed();

    // --- GET BY ID ---
    let start = Instant::now();
    for id in &task_ids {
        let _ = repo.get_by_id(*id).await.unwrap();
    }
    let get_duration = start.elapsed();

    // --- LIST ---
    let start = Instant::now();
    for _ in 0..LIST_ITERATIONS {
        let _ = repo
            .list(TaskFilter {
                limit: 50,
                offset: 0,
                ..Default::default()
            })
            .await
            .unwrap();
    }
    let list_duration = start.elapsed();

    // --- UPDATE ---
    let start = Instant::now();
    for id in &task_ids {
        let _ = repo
            .update(
                *id,
                domain_tasks::UpdateTask {
                    title: Some("Updated Title".to_string()),
                    status: Some(domain_tasks::TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    let update_duration = start.elapsed();

    // --- DELETE ---
    let start = Instant::now();
    for id in &task_ids {
        let _ = repo.delete(*id).await.unwrap();
    }
    let delete_duration = start.elapsed();

    // --- COUNT (on empty after delete) ---
    let count = repo.count().await.unwrap();
    assert_eq!(count, 0, "{name}: all tasks should be deleted");

    let result = BenchmarkResult {
        name: name.to_string(),
        create_ms: create_duration.as_millis() as f64,
        get_ms: get_duration.as_millis() as f64,
        list_ms: list_duration.as_millis() as f64,
        update_ms: update_duration.as_millis() as f64,
        delete_ms: delete_duration.as_millis() as f64,
        batch_size: BATCH_SIZE,
        list_iterations: LIST_ITERATIONS,
    };

    result.print();
    result
}

#[derive(Debug)]
struct BenchmarkResult {
    name: String,
    create_ms: f64,
    get_ms: f64,
    list_ms: f64,
    update_ms: f64,
    delete_ms: f64,
    batch_size: usize,
    list_iterations: usize,
}

impl BenchmarkResult {
    fn print(&self) {
        println!("\n=== {name} Performance ===", name = self.name);
        println!(
            "  CREATE  {n} tasks:  {ms:.1}ms  ({per:.2}ms/op)",
            n = self.batch_size,
            ms = self.create_ms,
            per = self.create_ms / self.batch_size as f64
        );
        println!(
            "  GET     {n} tasks:  {ms:.1}ms  ({per:.2}ms/op)",
            n = self.batch_size,
            ms = self.get_ms,
            per = self.get_ms / self.batch_size as f64
        );
        println!(
            "  LIST    {n} iters:  {ms:.1}ms  ({per:.2}ms/op)",
            n = self.list_iterations,
            ms = self.list_ms,
            per = self.list_ms / self.list_iterations as f64
        );
        println!(
            "  UPDATE  {n} tasks:  {ms:.1}ms  ({per:.2}ms/op)",
            n = self.batch_size,
            ms = self.update_ms,
            per = self.update_ms / self.batch_size as f64
        );
        println!(
            "  DELETE  {n} tasks:  {ms:.1}ms  ({per:.2}ms/op)",
            n = self.batch_size,
            ms = self.delete_ms,
            per = self.delete_ms / self.batch_size as f64
        );
        let total =
            self.create_ms + self.get_ms + self.list_ms + self.update_ms + self.delete_ms;
        println!("  TOTAL:              {total:.1}ms");
    }
}

#[cfg(feature = "mongo")]
fn print_comparison(a: &BenchmarkResult, b: &BenchmarkResult) {
    let sep = "=".repeat(60);
    println!("\n{sep}");
    println!("  COMPARISON: {} vs {}", a.name, b.name);
    println!("{sep}");

    let compare = |op: &str, a_ms: f64, b_ms: f64| {
        let ratio = if a_ms < b_ms {
            format!("{} {:.1}x faster", a.name, b_ms / a_ms)
        } else if b_ms < a_ms {
            format!("{} {:.1}x faster", b.name, a_ms / b_ms)
        } else {
            "Equal".to_string()
        };
        println!(
            "  {op:<10} {a_name}: {a_ms:>8.1}ms  {b_name}: {b_ms:>8.1}ms  => {ratio}",
            a_name = a.name,
            b_name = b.name,
        );
    };

    compare("CREATE", a.create_ms, b.create_ms);
    compare("GET", a.get_ms, b.get_ms);
    compare("LIST", a.list_ms, b.list_ms);
    compare("UPDATE", a.update_ms, b.update_ms);
    compare("DELETE", a.delete_ms, b.delete_ms);
}

// =============================================================================
// Test: PostgreSQL Performance (always available)
// =============================================================================

#[tokio::test]
async fn perf_postgres() {
    let db = TestDatabase::new().await;
    let repo = PgTaskRepository::new(db.connection());
    benchmark_repository("PostgreSQL", &repo).await;
}

// =============================================================================
// Test: PostgreSQL Direct baseline (always available)
// =============================================================================

#[tokio::test]
async fn perf_grpc_vs_direct_postgres() {
    let db = TestDatabase::new().await;

    let pg_repo = PgTaskRepository::new(db.connection());
    let direct_result = benchmark_repository("PostgreSQL (Direct)", &pg_repo).await;

    let total = direct_result.create_ms
        + direct_result.get_ms
        + direct_result.list_ms
        + direct_result.update_ms
        + direct_result.delete_ms;

    println!("\n=== gRPC vs Direct Comparison ===");
    println!("  Direct PostgreSQL total: {total:.1}ms");
    println!("  (gRPC adds serialization + HTTP/2 framing overhead)");
    println!("  To test full gRPC path, run the API with all three endpoints");
    println!("  and use a load testing tool like 'hey' or 'k6':");
    println!("    hey -n 1000 -c 10 http://localhost:8080/api/tasks-direct");
    println!("    hey -n 1000 -c 10 http://localhost:8080/api/tasks");
    println!("    hey -n 1000 -c 10 http://localhost:8080/api/tasks-mongo");
}

// =============================================================================
// MongoDB tests - only compiled when `mongo` feature is enabled
// =============================================================================

#[cfg(feature = "mongo")]
mod mongo_tests {
    use super::*;
    use domain_tasks::MongoTaskRepository;
    use test_utils::TestMongo;

    #[tokio::test]
    async fn perf_mongodb() {
        let mongo = TestMongo::new().await;
        let client = mongodb::Client::with_uri_str(&mongo.connection_string)
            .await
            .unwrap();
        let db = client.database(&mongo.database_name);
        let repo = MongoTaskRepository::new(db);
        benchmark_repository("MongoDB", &repo).await;
    }

    #[tokio::test]
    async fn perf_postgres_vs_mongodb() {
        let (pg_db, mongo) = tokio::join!(TestDatabase::new(), TestMongo::new());

        let pg_repo = PgTaskRepository::new(pg_db.connection());
        let mongo_client = mongodb::Client::with_uri_str(&mongo.connection_string)
            .await
            .unwrap();
        let mongo_db = mongo_client.database(&mongo.database_name);
        let mongo_repo = MongoTaskRepository::new(mongo_db);

        let pg_result = benchmark_repository("PostgreSQL", &pg_repo).await;
        let mongo_result = benchmark_repository("MongoDB", &mongo_repo).await;

        print_comparison(&pg_result, &mongo_result);
    }

    #[tokio::test]
    async fn perf_concurrent_creates() {
        let (pg_db, mongo) = tokio::join!(TestDatabase::new(), TestMongo::new());

        let concurrency = 10;
        let per_task = 10;

        // PostgreSQL concurrent creates
        let start = Instant::now();
        let mut handles = Vec::new();
        for batch in 0..concurrency {
            let repo = PgTaskRepository::new(pg_db.connection());
            handles.push(tokio::spawn(async move {
                for i in 0..per_task {
                    repo.create(create_sample_task(batch * per_task + i))
                        .await
                        .unwrap();
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let pg_concurrent_ms = start.elapsed().as_millis();

        // MongoDB concurrent creates
        let start = Instant::now();
        let mut handles = Vec::new();
        for batch in 0..concurrency {
            let client = mongodb::Client::with_uri_str(&mongo.connection_string)
                .await
                .unwrap();
            let db = client.database(&mongo.database_name);
            let repo = MongoTaskRepository::new(db);
            handles.push(tokio::spawn(async move {
                for i in 0..per_task {
                    repo.create(create_sample_task(batch * per_task + i))
                        .await
                        .unwrap();
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let mongo_concurrent_ms = start.elapsed().as_millis();

        let total_ops = concurrency * per_task;
        println!(
            "\n=== Concurrent Creates ({concurrency} workers x {per_task} tasks = {total_ops} total) ==="
        );
        println!(
            "  PostgreSQL:  {pg_concurrent_ms}ms  ({:.2}ms/op)",
            pg_concurrent_ms as f64 / total_ops as f64
        );
        println!(
            "  MongoDB:     {mongo_concurrent_ms}ms  ({:.2}ms/op)",
            mongo_concurrent_ms as f64 / total_ops as f64
        );

        if pg_concurrent_ms < mongo_concurrent_ms {
            println!(
                "  => PostgreSQL {:.1}x faster under concurrency",
                mongo_concurrent_ms as f64 / pg_concurrent_ms as f64
            );
        } else {
            println!(
                "  => MongoDB {:.1}x faster under concurrency",
                pg_concurrent_ms as f64 / mongo_concurrent_ms as f64
            );
        }
    }
}
