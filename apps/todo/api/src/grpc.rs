//! `todo.v1.TodoService` — the gRPC transport of the same backend.
//!
//! Mounted on the SAME listener as REST, SSE and WebSocket: tonic services are
//! plain `tower::Service`s, and `axum::serve` speaks h2c, so
//! `/todo.v1.TodoService/*` is just another route on `:8080`. Nothing here
//! owns state the HTTP routes do not — it wraps the identical [`TodoService`]
//! and subscribes to the identical database-sourced event bus
//! ([`crate::events`]), which is what makes it a fourth *transport* rather
//! than a second backend.
//!
//! Wire mapping (see `manifests/grpc/proto/apps/v1/todo.proto`): UUIDs travel
//! as text and timestamps as unix milliseconds; `PRIORITY_UNSPECIFIED` on
//! input means "domain default". Domain errors map onto status codes the
//! same way the REST layer maps them onto HTTP statuses.

use std::pin::Pin;

use chrono::{DateTime, Utc};
use domain_todo::{
    CreateTodo, Todo, TodoError, TodoEvent, TodoEventKind, TodoFilter, TodoPriority,
    TodoRepository, TodoService, UpdateTodo,
};
use futures::Stream;
use futures::StreamExt;
use rpc::todo::v1::todo_service_server::{SERVICE_NAME, TodoServiceServer};
use rpc::todo::v1::{self as pb, todo_service_server};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tonic::codec::CompressionEncoding;
use tonic::service::Routes;
use tonic::{Request, Response, Status};
use tonic_health::ServingStatus;
use uuid::Uuid;

type EventStream = Pin<Box<dyn Stream<Item = Result<pb::TodoEvent, Status>> + Send>>;

/// Build the gRPC routes as an [`axum::Router`] to merge into the HTTP app.
///
/// Registers `grpc.health.v1.Health` alongside the service so `grpcurl` and
/// kubelet's native gRPC prober can ask "is `todo.v1.TodoService` serving?".
/// Starts from an EMPTY axum router, not [`Routes::default`], so no
/// `UNIMPLEMENTED` fallback is merged over the REST routes.
pub async fn router<R: TodoRepository + 'static>(
    service: TodoService<R>,
    events: broadcast::Sender<TodoEvent>,
) -> axum::Router {
    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_service_status(SERVICE_NAME, ServingStatus::Serving)
        .await;

    let grpc = TodoServiceServer::new(TodoGrpc { service, events })
        .accept_compressed(CompressionEncoding::Zstd)
        .send_compressed(CompressionEncoding::Zstd);

    Routes::from(axum::Router::new())
        .add_service(health_service)
        .add_service(grpc)
        .into_axum_router()
}

struct TodoGrpc<R: TodoRepository> {
    service: TodoService<R>,
    events: broadcast::Sender<TodoEvent>,
}

/// Same policy as `TodoError -> AppError` on the REST side: not-found and
/// validation are the caller's, everything else is ours and stays opaque.
fn to_status(err: TodoError) -> Status {
    match err {
        TodoError::NotFound(id) => Status::not_found(format!("todo {id} not found")),
        TodoError::Validation(msg) => Status::invalid_argument(msg),
        TodoError::Internal(msg) => Status::internal(msg),
        TodoError::Database(e) => {
            tracing::error!(error = %e, "database error");
            Status::internal("database error")
        }
    }
}

fn parse_id(id: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(id).map_err(|_| Status::invalid_argument(format!("invalid uuid: {id:?}")))
}

fn priority_from_pb(raw: i32) -> Result<Option<TodoPriority>, Status> {
    match pb::Priority::try_from(raw) {
        Ok(pb::Priority::Unspecified) => Ok(None),
        Ok(pb::Priority::Low) => Ok(Some(TodoPriority::Low)),
        Ok(pb::Priority::Medium) => Ok(Some(TodoPriority::Medium)),
        Ok(pb::Priority::High) => Ok(Some(TodoPriority::High)),
        Err(_) => Err(Status::invalid_argument(format!("unknown priority {raw}"))),
    }
}

fn priority_to_pb(priority: TodoPriority) -> pb::Priority {
    match priority {
        TodoPriority::Low => pb::Priority::Low,
        TodoPriority::Medium => pb::Priority::Medium,
        TodoPriority::High => pb::Priority::High,
    }
}

fn kind_to_pb(kind: TodoEventKind) -> pb::EventKind {
    match kind {
        TodoEventKind::Created => pb::EventKind::Created,
        TodoEventKind::Updated => pb::EventKind::Updated,
        TodoEventKind::Completed => pb::EventKind::Completed,
        TodoEventKind::Uncompleted => pb::EventKind::Uncompleted,
        TodoEventKind::Deleted => pb::EventKind::Deleted,
    }
}

fn millis(at: DateTime<Utc>) -> i64 {
    at.timestamp_millis()
}

fn todo_to_pb(todo: Todo) -> pb::Todo {
    pb::Todo {
        id: todo.id.to_string(),
        title: todo.title,
        description: todo.description,
        completed: todo.completed,
        priority: priority_to_pb(todo.priority) as i32,
        created_at_ms: millis(todo.created_at),
        updated_at_ms: millis(todo.updated_at),
    }
}

fn event_to_pb(event: TodoEvent) -> pb::TodoEvent {
    pb::TodoEvent {
        event_id: event.event_id.to_string(),
        kind: kind_to_pb(event.kind) as i32,
        todo_id: event.todo_id.to_string(),
        todo: event.todo.map(todo_to_pb),
        occurred_at_ms: millis(event.occurred_at),
    }
}

fn reply(todo: Todo) -> Result<Response<pb::Todo>, Status> {
    Ok(Response::new(todo_to_pb(todo)))
}

#[tonic::async_trait]
impl<R: TodoRepository + 'static> todo_service_server::TodoService for TodoGrpc<R> {
    async fn list(
        &self,
        request: Request<pb::ListRequest>,
    ) -> Result<Response<pb::ListResponse>, Status> {
        let req = request.into_inner();
        let defaults = TodoFilter::default();
        let filter = TodoFilter {
            completed: req.completed,
            priority: req.priority.map(priority_from_pb).transpose()?.flatten(),
            limit: if req.limit == 0 {
                defaults.limit
            } else {
                req.limit as usize
            },
            offset: req.offset as usize,
        };
        let todos = self.service.list_todos(filter).await.map_err(to_status)?;
        Ok(Response::new(pb::ListResponse {
            todos: todos.into_iter().map(todo_to_pb).collect(),
        }))
    }

    async fn get(&self, request: Request<pb::GetRequest>) -> Result<Response<pb::Todo>, Status> {
        let id = parse_id(&request.into_inner().id)?;
        reply(self.service.get_todo(id).await.map_err(to_status)?)
    }

    async fn create(
        &self,
        request: Request<pb::CreateRequest>,
    ) -> Result<Response<pb::Todo>, Status> {
        let req = request.into_inner();
        let input = CreateTodo {
            title: req.title,
            description: req.description,
            priority: priority_from_pb(req.priority)?.unwrap_or_default(),
        };
        reply(self.service.create_todo(input).await.map_err(to_status)?)
    }

    async fn update(
        &self,
        request: Request<pb::UpdateRequest>,
    ) -> Result<Response<pb::Todo>, Status> {
        let req = request.into_inner();
        let id = parse_id(&req.id)?;
        let input = UpdateTodo {
            title: req.title,
            description: req.description,
            completed: req.completed,
            priority: req.priority.map(priority_from_pb).transpose()?.flatten(),
        };
        reply(
            self.service
                .update_todo(id, input)
                .await
                .map_err(to_status)?,
        )
    }

    async fn delete(
        &self,
        request: Request<pb::DeleteRequest>,
    ) -> Result<Response<pb::DeleteResponse>, Status> {
        let id = parse_id(&request.into_inner().id)?;
        self.service.delete_todo(id).await.map_err(to_status)?;
        Ok(Response::new(pb::DeleteResponse {}))
    }

    async fn complete(
        &self,
        request: Request<pb::CompleteRequest>,
    ) -> Result<Response<pb::Todo>, Status> {
        let id = parse_id(&request.into_inner().id)?;
        reply(self.service.complete_todo(id).await.map_err(to_status)?)
    }

    async fn uncomplete(
        &self,
        request: Request<pb::UncompleteRequest>,
    ) -> Result<Response<pb::Todo>, Status> {
        let id = parse_id(&request.into_inner().id)?;
        reply(self.service.uncomplete_todo(id).await.map_err(to_status)?)
    }

    type WatchStream = EventStream;

    /// A slow consumer that lags the bus gets `RESOURCE_EXHAUSTED` with the
    /// drop count and the stream ends — the gRPC spelling of the SSE
    /// `lagged` event, and a signal to re-`List` and re-`Watch`.
    async fn watch(
        &self,
        _request: Request<pb::WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        let stream = BroadcastStream::new(self.events.subscribe()).map(|item| match item {
            Ok(event) => Ok(event_to_pb(event)),
            Err(BroadcastStreamRecvError::Lagged(skipped)) => Err(Status::resource_exhausted(
                format!("lagged: {skipped} events dropped"),
            )),
        });
        Ok(Response::new(Box::pin(stream)))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use domain_todo::{NoopTodoPublisher, TodoResult};
    use parking_lot::Mutex;
    use rpc::todo::v1::todo_service_client::TodoServiceClient;
    use tonic::transport::Channel;

    use super::*;

    /// The smallest honest repository: a locked Vec, so the test exercises
    /// the real service (validation, event emission) over a real socket.
    #[derive(Default)]
    struct MemRepo(Mutex<Vec<Todo>>);

    #[async_trait]
    impl TodoRepository for MemRepo {
        async fn create(&self, input: CreateTodo) -> TodoResult<Todo> {
            let now = Utc::now();
            let todo = Todo {
                id: Uuid::now_v7(),
                title: input.title,
                description: input.description,
                completed: false,
                priority: input.priority,
                created_at: now,
                updated_at: now,
            };
            self.0.lock().push(todo.clone());
            Ok(todo)
        }
        async fn get_by_id(&self, id: Uuid) -> TodoResult<Option<Todo>> {
            Ok(self.0.lock().iter().find(|t| t.id == id).cloned())
        }
        async fn list(&self, filter: TodoFilter) -> TodoResult<Vec<Todo>> {
            Ok(self
                .0
                .lock()
                .iter()
                .filter(|t| filter.completed.is_none_or(|c| c == t.completed))
                .filter(|t| filter.priority.is_none_or(|p| p == t.priority))
                .skip(filter.offset)
                .take(filter.limit)
                .cloned()
                .collect())
        }
        async fn update(&self, id: Uuid, input: UpdateTodo) -> TodoResult<Todo> {
            let mut todos = self.0.lock();
            let todo = todos
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or(TodoError::NotFound(id))?;
            todo.apply_update(input);
            Ok(todo.clone())
        }
        async fn delete(&self, id: Uuid) -> TodoResult<bool> {
            let mut todos = self.0.lock();
            let before = todos.len();
            todos.retain(|t| t.id != id);
            Ok(todos.len() != before)
        }
        async fn count(&self) -> TodoResult<usize> {
            Ok(self.0.lock().len())
        }
    }

    /// Serve REST + gRPC on one ephemeral port exactly as `main` does, so the
    /// test proves the multiplexing, not just the handlers.
    async fn serve() -> (
        TodoServiceClient<Channel>,
        String,
        broadcast::Sender<TodoEvent>,
    ) {
        let service = TodoService::new(MemRepo::default(), Arc::new(NoopTodoPublisher));
        let events = crate::events::channel();
        let app = axum::Router::new()
            .route("/healthz", axum::routing::get(|| async { "ok" }))
            .nest("/api/todos", domain_todo::router(service.clone()))
            .merge(router(service, events.clone()).await);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = TodoServiceClient::connect(format!("http://{addr}"))
            .await
            .unwrap();
        (client, format!("http://{addr}"), events)
    }

    #[tokio::test]
    async fn crud_round_trip_over_grpc_is_visible_over_rest() {
        let (mut client, base, _events) = serve().await;

        let created = client
            .create(pb::CreateRequest {
                title: "over grpc".into(),
                description: String::new(),
                priority: pb::Priority::High as i32,
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(created.priority, pb::Priority::High as i32);
        assert!(!created.completed);

        let done = client
            .complete(pb::CompleteRequest {
                id: created.id.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        assert!(done.completed);
        assert!(done.updated_at_ms >= created.updated_at_ms);

        let listed = client
            .list(pb::ListRequest {
                completed: Some(true),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(listed.todos.len(), 1);
        assert_eq!(listed.todos[0].id, created.id);

        // Same process, same port, same store: REST sees the gRPC write.
        let json: serde_json::Value =
            reqwest_get(&format!("{base}/api/todos/{}", created.id)).await;
        assert_eq!(json["title"], "over grpc");
        assert_eq!(json["completed"], true);

        client
            .delete(pb::DeleteRequest {
                id: created.id.clone(),
            })
            .await
            .unwrap();
        let missing = client
            .get(pb::GetRequest { id: created.id })
            .await
            .unwrap_err();
        assert_eq!(missing.code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn domain_validation_maps_to_invalid_argument() {
        let (mut client, _, _) = serve().await;

        let err = client
            .create(pb::CreateRequest {
                title: String::new(),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);

        let err = client
            .get(pb::GetRequest {
                id: "not-a-uuid".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn watch_streams_bus_events() {
        let (mut client, _, events) = serve().await;

        let mut stream = client
            .watch(pb::WatchRequest {})
            .await
            .unwrap()
            .into_inner();

        // The subscriber exists once the response is back; publish after it.
        let event = TodoEvent::deleted(Uuid::now_v7());
        events.send(event.clone()).expect("subscriber exists");

        let got = stream.message().await.unwrap().expect("one event");
        assert_eq!(got.kind, pb::EventKind::Deleted as i32);
        assert_eq!(got.todo_id, event.todo_id.to_string());
        assert!(got.todo.is_none());
    }

    #[tokio::test]
    async fn health_reports_service_serving() {
        let (_, base, _) = serve().await;
        let channel = Channel::from_shared(base).unwrap().connect().await.unwrap();
        let mut health = tonic_health::pb::health_client::HealthClient::new(channel);
        let status = health
            .check(tonic_health::pb::HealthCheckRequest {
                service: SERVICE_NAME.into(),
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            status.status,
            tonic_health::pb::health_check_response::ServingStatus::Serving as i32
        );
    }

    /// tonic's `Channel` is h2-only; the REST check needs a plain HTTP/1.1 client.
    async fn reqwest_get(url: &str) -> serde_json::Value {
        let body = reqwest::get(url).await.unwrap().bytes().await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }
}
