//! Restate <-> endpoint <-> NATS integration test.
//!
//! A real Restate server (container) drives `TodoObject` served by this test
//! process through its ingress, and every accepted lifecycle step must land on
//! the `todos.>` subjects `todo_worker` consumes, in order.
//!
//! Requires Docker. Run: `cargo nextest run -p todo_restate --test lifecycle_it`.

use std::sync::Arc;
use std::time::Duration;

use domain_todo::{
    CreateTodo, NatsTodoPublisher, Todo, TodoEvent, TodoEventKind, TodoPriority, UpdateTodo,
};
use futures::StreamExt;
use restate_sdk::ingress::{CallResponse, ClientError, ReqwestClient};
use restate_sdk::prelude::{Endpoint, HttpServer, Json};
use test_utils::{TestNats, TestRestate};
use todo_restate::{TodoObject, TodoObjectIngressClient};
use tokio::net::TcpListener;
use uuid::Uuid;

/// HTTP status of an ingress call. A handler's `TerminalError` comes back as a
/// completed call carrying the error code, not as a transport `Err`.
fn status<E, R>(result: Result<CallResponse<E, R>, ClientError>) -> u16 {
    match result {
        Ok(r) => r.status().as_u16(),
        Err(e) => e
            .response()
            .unwrap_or_else(|| panic!("no HTTP response: {e}"))
            .status()
            .as_u16(),
    }
}

fn create(title: &str) -> Json<CreateTodo> {
    Json(CreateTodo {
        title: title.into(),
        description: String::new(),
        priority: TodoPriority::Low,
    })
}

#[tokio::test]
async fn todo_lifecycle_keeps_state_in_restate_and_publishes_each_step() {
    let nats = TestNats::new().await;
    let mut events = nats
        .client()
        .subscribe("todos.>")
        .await
        .expect("subscribe todos.>");
    let publisher = NatsTodoPublisher::new(nats.jetstream())
        .await
        .expect("init publisher + stream");

    let listener = TcpListener::bind("0.0.0.0:0").await.expect("bind endpoint");
    let port = listener.local_addr().expect("endpoint addr").port();
    let endpoint = Endpoint::builder()
        .bind(TodoObject::new(Arc::new(publisher)))
        .build();
    let server = tokio::spawn(HttpServer::new(endpoint).serve(listener));

    let restate = TestRestate::new().await;
    restate.register_host_endpoint(port).await;
    let client = ReqwestClient::connect(restate.ingress_url().parse().expect("ingress uri"))
        .expect("client");

    let id = Uuid::now_v7();
    let todo = TodoObjectIngressClient::from_client(client.clone(), id.to_string());

    assert_eq!(status(todo.get().call().await), 404, "get before create");

    let created: Todo = todo
        .create(create("try restate"))
        .call()
        .await
        .expect("create")
        .into_body()
        .expect("create body")
        .into_inner();
    assert_eq!(created.id, id, "the object key is the todo id");
    assert!(!created.completed);
    assert_eq!(
        status(todo.create(create("again")).call().await),
        409,
        "create is not an upsert"
    );
    assert_eq!(
        status(todo.create(create("")).call().await),
        400,
        "empty title"
    );

    let updated: Todo = todo
        .update(Json(UpdateTodo {
            priority: Some(TodoPriority::High),
            ..Default::default()
        }))
        .call()
        .await
        .expect("update")
        .into_body()
        .expect("update body")
        .into_inner();
    assert_eq!(updated.priority, TodoPriority::High);
    assert_eq!(updated.title, "try restate", "PATCH keeps untouched fields");
    assert!(updated.updated_at >= created.updated_at);

    let completed: Todo = todo
        .complete()
        .call()
        .await
        .expect("complete")
        .into_body()
        .expect("complete body")
        .into_inner();
    assert!(completed.completed);

    let read: Todo = todo
        .get()
        .call()
        .await
        .expect("get")
        .into_body()
        .expect("get body")
        .into_inner();
    assert_eq!(read, completed, "state survives across invocations");

    todo.delete()
        .call()
        .await
        .expect("delete")
        .into_body()
        .expect("delete body");
    assert_eq!(status(todo.get().call().await), 404, "get after delete");
    assert_eq!(status(todo.delete().call().await), 404, "delete twice");

    let bad_key = TodoObjectIngressClient::from_client(client, "not-a-uuid");
    assert_eq!(
        status(bad_key.create(create("x")).call().await),
        400,
        "non-UUID key"
    );

    let mut kinds = Vec::new();
    for _ in 0..4 {
        let msg = tokio::time::timeout(Duration::from_secs(10), events.next())
            .await
            .expect("event within timeout")
            .expect("subscription open");
        let event: TodoEvent = serde_json::from_slice(&msg.payload).expect("decode TodoEvent");
        assert_eq!(event.todo_id, id);
        assert_eq!(msg.subject.as_str(), event.subject());
        kinds.push(event.kind);
    }
    assert_eq!(
        kinds,
        [
            TodoEventKind::Created,
            TodoEventKind::Updated,
            TodoEventKind::Completed,
            TodoEventKind::Deleted,
        ],
        "one event per accepted step; rejected calls publish nothing"
    );

    server.abort();
}
