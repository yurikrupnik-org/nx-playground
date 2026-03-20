//! Async task endpoints that publish DB operations via Dapr pub/sub.
//!
//! Instead of hitting the database directly, these endpoints publish
//! `DbOpEvent`s to the Dapr pub/sub topic. The db-worker service
//! picks them up and executes the actual database operations.
//!
//! This enables:
//! - Decoupled DB backend selection (PG, Mongo, etc.) without API changes
//! - Event-driven processing with retry/DLQ semantics
//! - Independent scaling of read vs. write workloads

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router, routing::get};
use dapr_client::PubSubClient;
use messaging::{DbOpEvent, DbOperation};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Shared state for async task handlers.
#[derive(Clone)]
pub struct AsyncTasksState {
    pub pubsub: PubSubClient,
}

/// Response returned when an async operation is accepted.
#[derive(Serialize)]
pub struct AsyncAccepted {
    pub event_id: String,
    pub status: &'static str,
    pub message: String,
}

/// Query params for choosing which DB backend to target.
#[derive(Debug, Deserialize)]
pub struct BackendQuery {
    /// Target backend: "pg" or "mongo" (default: "pg")
    #[serde(default = "default_backend")]
    pub backend: String,
}

fn default_backend() -> String {
    "pg".to_string()
}

impl BackendQuery {
    fn topic(&self) -> &str {
        match self.backend.as_str() {
            "mongo" => "db.tasks.mongo",
            _ => "db.tasks.pg",
        }
    }
}

pub fn router(state: &crate::state::AppState) -> Option<Router> {
    let pubsub = state.pubsub.as_ref()?;

    let async_state = AsyncTasksState {
        pubsub: pubsub.clone(),
    };

    Some(
        Router::new()
            .route("/", get(list_tasks).post(create_task))
            .route("/{id}", get(get_task).put(update_task).delete(delete_task))
            .with_state(async_state),
    )
}

/// POST /tasks-async?backend=pg
///
/// Publishes a Create operation. Returns 202 Accepted immediately.
async fn create_task(
    State(state): State<AsyncTasksState>,
    Query(backend): Query<BackendQuery>,
    Json(input): Json<serde_json::Value>,
) -> impl IntoResponse {
    let event = DbOpEvent::new(DbOperation::Create, "tasks", input);
    let event_id = event.id.clone();
    let topic = backend.topic();

    match state.pubsub.publish(topic, &event).await {
        Ok(()) => {
            info!(event_id = %event_id, topic, "Task create event published");
            (
                StatusCode::ACCEPTED,
                Json(AsyncAccepted {
                    event_id,
                    status: "accepted",
                    message: format!("Create task queued for processing (topic: {})", topic),
                }),
            )
        }
        Err(e) => {
            warn!(error = %e, "Failed to publish create event");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(AsyncAccepted {
                    event_id,
                    status: "error",
                    message: format!("Failed to queue task: {}", e),
                }),
            )
        }
    }
}

/// GET /tasks-async?backend=pg
///
/// Publishes a Query operation.
async fn list_tasks(
    State(state): State<AsyncTasksState>,
    Query(backend): Query<BackendQuery>,
) -> impl IntoResponse {
    let event = DbOpEvent::new(
        DbOperation::Query,
        "tasks",
        serde_json::json!({"filter": {}}),
    );
    let event_id = event.id.clone();
    let topic = backend.topic();

    match state.pubsub.publish(topic, &event).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(AsyncAccepted {
                event_id,
                status: "accepted",
                message: format!("List tasks queued (topic: {})", topic),
            }),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(AsyncAccepted {
                event_id,
                status: "error",
                message: format!("Failed to queue query: {}", e),
            }),
        ),
    }
}

/// GET /tasks-async/:id?backend=pg
///
/// Publishes a Read operation for a specific task.
async fn get_task(
    State(state): State<AsyncTasksState>,
    Path(id): Path<String>,
    Query(backend): Query<BackendQuery>,
) -> impl IntoResponse {
    let event = DbOpEvent::new(
        DbOperation::Read,
        "tasks",
        serde_json::json!({"id": id}),
    );
    let event_id = event.id.clone();
    let topic = backend.topic();

    match state.pubsub.publish(topic, &event).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(AsyncAccepted {
                event_id,
                status: "accepted",
                message: format!("Get task {} queued (topic: {})", id, topic),
            }),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(AsyncAccepted {
                event_id,
                status: "error",
                message: format!("Failed to queue read: {}", e),
            }),
        ),
    }
}

/// PUT /tasks-async/:id?backend=pg
///
/// Publishes an Update operation.
async fn update_task(
    State(state): State<AsyncTasksState>,
    Path(id): Path<String>,
    Query(backend): Query<BackendQuery>,
    Json(mut input): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Inject the ID into the payload so the worker knows which entity to update
    if let Some(obj) = input.as_object_mut() {
        obj.insert("id".to_string(), serde_json::Value::String(id.clone()));
    }

    let event = DbOpEvent::new(DbOperation::Update, "tasks", input);
    let event_id = event.id.clone();
    let topic = backend.topic();

    match state.pubsub.publish(topic, &event).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(AsyncAccepted {
                event_id,
                status: "accepted",
                message: format!("Update task {} queued (topic: {})", id, topic),
            }),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(AsyncAccepted {
                event_id,
                status: "error",
                message: format!("Failed to queue update: {}", e),
            }),
        ),
    }
}

/// DELETE /tasks-async/:id?backend=pg
///
/// Publishes a Delete operation.
async fn delete_task(
    State(state): State<AsyncTasksState>,
    Path(id): Path<String>,
    Query(backend): Query<BackendQuery>,
) -> impl IntoResponse {
    let event = DbOpEvent::new(
        DbOperation::Delete,
        "tasks",
        serde_json::json!({"id": id}),
    );
    let event_id = event.id.clone();
    let topic = backend.topic();

    match state.pubsub.publish(topic, &event).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(AsyncAccepted {
                event_id,
                status: "accepted",
                message: format!("Delete task {} queued (topic: {})", id, topic),
            }),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(AsyncAccepted {
                event_id,
                status: "error",
                message: format!("Failed to queue delete: {}", e),
            }),
        ),
    }
}
