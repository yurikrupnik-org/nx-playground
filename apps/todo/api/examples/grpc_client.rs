//! Drive `todo.v1.TodoService` end to end against a running `todo_api`:
//! subscribe to `Watch`, create → complete → delete over gRPC, and print the
//! three events the database trigger sends back. Every event arrives via
//! Postgres `NOTIFY`, not the request path — the same bus the SSE/WebSocket
//! clients read.
//!
//! ```bash
//! just run todo-api                      # or: cargo run -p todo_api
//! cargo run -p todo_api --example grpc_client [http://localhost:8080]
//! ```

use rpc::todo::v1::todo_service_client::TodoServiceClient;
use rpc::todo::v1::{
    CompleteRequest, CreateRequest, DeleteRequest, EventKind, ListRequest, Priority, WatchRequest,
};
use tokio::time::{Duration, timeout};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://localhost:8080".to_string());
    let mut client = TodoServiceClient::connect(endpoint.clone()).await?;
    println!("connected {endpoint}");

    // Subscribe before writing so nothing is missed (NOTIFY has no backlog).
    let mut watch = client.watch(WatchRequest {}).await?.into_inner();

    let created = client
        .create(CreateRequest {
            title: "from grpc_client example".into(),
            description: String::new(),
            priority: Priority::High as i32,
        })
        .await?
        .into_inner();
    println!("created {} priority={:?}", created.id, created.priority());

    let done = client
        .complete(CompleteRequest {
            id: created.id.clone(),
        })
        .await?
        .into_inner();
    println!("completed {} completed={}", done.id, done.completed);

    let listed = client
        .list(ListRequest {
            completed: Some(true),
            ..Default::default()
        })
        .await?
        .into_inner();
    println!("list completed=true -> {} todos", listed.todos.len());

    client
        .delete(DeleteRequest {
            id: created.id.clone(),
        })
        .await?;
    println!("deleted {}", created.id);

    // The trigger classifies the three writes as created/completed/deleted.
    for expected in [EventKind::Created, EventKind::Completed, EventKind::Deleted] {
        let event = timeout(Duration::from_secs(5), watch.message())
            .await
            .map_err(|_| "timed out waiting for a Watch event")??
            .ok_or("Watch stream ended early")?;
        println!(
            "event {:?} todo_id={} snapshot={}",
            event.kind(),
            event.todo_id,
            event.todo.is_some()
        );
        assert_eq!(event.kind(), expected);
        assert_eq!(event.todo_id, created.id);
    }
    Ok(())
}
