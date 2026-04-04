//! gRPC vs Direct API Performance Test
//!
//! Spins up a real PostgreSQL container, starts an in-process gRPC server,
//! and compares performance of:
//! - Direct repository calls (PgTaskRepository)
//! - gRPC client -> server -> PgTaskRepository roundtrip
//! - MongoDB direct repository calls (MongoTaskRepository)
//!
//! Run with: cargo test -p zerg_tasks --test grpc_perf_test -- --nocapture

use domain_tasks::{
    CreateTask, MongoTaskRepository, PgTaskRepository, TaskFilter, TaskRepository, TaskService,
};
use rpc::tasks::tasks_service_client::TasksServiceClient;
use rpc::tasks::tasks_service_server::TasksServiceServer;
use rpc::tasks::{CreateRequest, DeleteByIdRequest, GetByIdRequest, ListRequest, UpdateByIdRequest};
use std::time::Instant;
use test_utils::{TestDatabase, TestMongo};
use tonic::codec::CompressionEncoding;
use tonic::transport::{Channel, Server};
use zerg_tasks::TasksServiceImpl;

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

/// Benchmark direct repository operations
async fn benchmark_direct<R: TaskRepository>(name: &str, repo: &R) -> BenchResult {
    // CREATE
    let start = Instant::now();
    let mut ids = Vec::with_capacity(BATCH_SIZE);
    for i in 0..BATCH_SIZE {
        let task = repo.create(create_sample_task(i)).await.unwrap();
        ids.push(task.id);
    }
    let create_ms = start.elapsed().as_millis() as f64;

    // GET
    let start = Instant::now();
    for id in &ids {
        repo.get_by_id(*id).await.unwrap();
    }
    let get_ms = start.elapsed().as_millis() as f64;

    // LIST
    let start = Instant::now();
    for _ in 0..LIST_ITERATIONS {
        repo.list(TaskFilter {
            limit: 50,
            ..Default::default()
        })
        .await
        .unwrap();
    }
    let list_ms = start.elapsed().as_millis() as f64;

    // UPDATE
    let start = Instant::now();
    for id in &ids {
        repo.update(
            *id,
            domain_tasks::UpdateTask {
                title: Some("Updated".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    let update_ms = start.elapsed().as_millis() as f64;

    // DELETE
    let start = Instant::now();
    for id in &ids {
        repo.delete(*id).await.unwrap();
    }
    let delete_ms = start.elapsed().as_millis() as f64;

    let result = BenchResult {
        name: name.to_string(),
        create_ms,
        get_ms,
        list_ms,
        update_ms,
        delete_ms,
    };
    result.print();
    result
}

/// Benchmark gRPC client operations (full roundtrip)
async fn benchmark_grpc(client: &mut TasksServiceClient<Channel>) -> BenchResult {
    // CREATE
    let start = Instant::now();
    let mut ids = Vec::with_capacity(BATCH_SIZE);
    for i in 0..BATCH_SIZE {
        let resp = client
            .create(CreateRequest {
                title: format!("gRPC Perf Task {i}"),
                description: format!("Description {i}"),
                project_id: None,
                priority: 2, // Medium
                status: 1,   // Todo
                due_date: None,
            })
            .await
            .unwrap();
        ids.push(resp.into_inner().id);
    }
    let create_ms = start.elapsed().as_millis() as f64;

    // GET
    let start = Instant::now();
    for id in &ids {
        client
            .get_by_id(GetByIdRequest { id: id.clone() })
            .await
            .unwrap();
    }
    let get_ms = start.elapsed().as_millis() as f64;

    // LIST
    let start = Instant::now();
    for _ in 0..LIST_ITERATIONS {
        client
            .list(ListRequest {
                project_id: None,
                status: None,
                priority: None,
                completed: None,
                limit: 50,
                offset: 0,
            })
            .await
            .unwrap();
    }
    let list_ms = start.elapsed().as_millis() as f64;

    // UPDATE
    let start = Instant::now();
    for id in &ids {
        client
            .update_by_id(UpdateByIdRequest {
                id: id.clone(),
                title: Some("Updated via gRPC".to_string()),
                description: None,
                completed: None,
                project_id: None,
                priority: None,
                status: None,
                due_date: None,
            })
            .await
            .unwrap();
    }
    let update_ms = start.elapsed().as_millis() as f64;

    // DELETE
    let start = Instant::now();
    for id in &ids {
        client
            .delete_by_id(DeleteByIdRequest { id: id.clone() })
            .await
            .unwrap();
    }
    let delete_ms = start.elapsed().as_millis() as f64;

    let result = BenchResult {
        name: "PostgreSQL (gRPC)".to_string(),
        create_ms,
        get_ms,
        list_ms,
        update_ms,
        delete_ms,
    };
    result.print();
    result
}

#[derive(Debug)]
struct BenchResult {
    name: String,
    create_ms: f64,
    get_ms: f64,
    list_ms: f64,
    update_ms: f64,
    delete_ms: f64,
}

impl BenchResult {
    fn total(&self) -> f64 {
        self.create_ms + self.get_ms + self.list_ms + self.update_ms + self.delete_ms
    }

    fn print(&self) {
        println!("\n=== {} Performance ===", self.name);
        println!(
            "  CREATE  {BATCH_SIZE} tasks:  {:.1}ms  ({:.2}ms/op)",
            self.create_ms,
            self.create_ms / BATCH_SIZE as f64
        );
        println!(
            "  GET     {BATCH_SIZE} tasks:  {:.1}ms  ({:.2}ms/op)",
            self.get_ms,
            self.get_ms / BATCH_SIZE as f64
        );
        println!(
            "  LIST    {LIST_ITERATIONS} iters:  {:.1}ms  ({:.2}ms/op)",
            self.list_ms,
            self.list_ms / LIST_ITERATIONS as f64
        );
        println!(
            "  UPDATE  {BATCH_SIZE} tasks:  {:.1}ms  ({:.2}ms/op)",
            self.update_ms,
            self.update_ms / BATCH_SIZE as f64
        );
        println!(
            "  DELETE  {BATCH_SIZE} tasks:  {:.1}ms  ({:.2}ms/op)",
            self.delete_ms,
            self.delete_ms / BATCH_SIZE as f64
        );
        println!("  TOTAL:              {:.1}ms", self.total());
    }
}

fn print_comparison(results: &[&BenchResult]) {
    let sep = "=".repeat(70);
    println!("\n{sep}");
    println!("  PERFORMANCE COMPARISON SUMMARY");
    println!("{sep}");

    let ops = ["CREATE", "GET", "LIST", "UPDATE", "DELETE", "TOTAL"];
    let get_val = |r: &BenchResult, op: &str| match op {
        "CREATE" => r.create_ms,
        "GET" => r.get_ms,
        "LIST" => r.list_ms,
        "UPDATE" => r.update_ms,
        "DELETE" => r.delete_ms,
        "TOTAL" => r.total(),
        _ => 0.0,
    };

    // Header
    print!("  {:<10}", "OP");
    for r in results {
        print!("  {:>18}", r.name);
    }
    println!();
    println!("  {}", "-".repeat(10 + results.len() * 20));

    for op in ops {
        print!("  {op:<10}");
        let vals: Vec<f64> = results.iter().map(|r| get_val(r, op)).collect();
        let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
        for v in &vals {
            if (*v - min).abs() < 0.01 {
                print!("  {:>14.1}ms *", v);
            } else {
                let ratio = v / min;
                print!("  {:>10.1}ms {:.1}x", v, ratio);
            }
        }
        println!();
    }
    println!("\n  * = fastest");
}

// =============================================================================
// Full comparison: PostgreSQL Direct vs gRPC vs MongoDB
// =============================================================================

#[tokio::test]
async fn perf_all_backends() {
    // Start containers
    let (pg_db, mongo) = tokio::join!(TestDatabase::new(), TestMongo::new());

    // --- PostgreSQL Direct ---
    let pg_repo = PgTaskRepository::new(pg_db.connection());
    let pg_direct = benchmark_direct("PG Direct", &pg_repo).await;

    // --- Start in-process gRPC server backed by PostgreSQL ---
    let grpc_pg_repo = PgTaskRepository::new(pg_db.connection());
    let task_service = TaskService::new(grpc_pg_repo);
    let tasks_grpc = TasksServiceServer::new(TasksServiceImpl::new(task_service))
        .accept_compressed(CompressionEncoding::Zstd)
        .send_compressed(CompressionEncoding::Zstd);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Convert to tonic's incoming stream
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    tokio::spawn(async move {
        Server::builder()
            .add_service(tasks_grpc)
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    // Give server a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Connect gRPC client
    let mut grpc_client =
        TasksServiceClient::connect(format!("http://127.0.0.1:{}", addr.port()))
            .await
            .unwrap();

    let grpc_result = benchmark_grpc(&mut grpc_client).await;

    // --- MongoDB Direct ---
    let mongo_client = mongodb::Client::with_uri_str(&mongo.connection_string)
        .await
        .unwrap();
    let mongo_db = mongo_client.database(&mongo.database_name);
    let mongo_repo = MongoTaskRepository::new(mongo_db);
    let mongo_direct = benchmark_direct("Mongo Direct", &mongo_repo).await;

    // --- Summary ---
    print_comparison(&[&pg_direct, &grpc_result, &mongo_direct]);
}
