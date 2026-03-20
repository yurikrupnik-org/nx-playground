//! Shared task operation dispatcher and fallback Dapr state store handler.
//!
//! `dispatch_task_op` is the core function used by both `tasks_pg` and `tasks_mongo`.
//! It deserializes the event payload into domain DTOs (`CreateTask`, `UpdateTask`, etc.)
//! and calls the `TaskService` directly.
//!
//! The `DaprStateHandler` is kept as a fallback for non-task entity types that can
//! use Dapr's generic key-value state store API.

use axum::Json;
use axum::extract::State;
use dapr_client::state::StateClient;
use dapr_client::subscription::{CloudEvent, DaprEventResponse};
use domain_tasks::{CreateTask, TaskFilter, TaskRepository, TaskService, UpdateTask};
use messaging::jobs::DbOperation;
use messaging::{DbOpEvent, Job};
use std::sync::Arc;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

/// Dispatch a task operation to the TaskService.
///
/// This is backend-agnostic: it works with any `TaskRepository` implementation
/// (PgTaskRepository, MongoTaskRepository, etc.).
pub async fn dispatch_task_op<R: TaskRepository>(
    service: &Arc<TaskService<R>>,
    event: DbOpEvent,
) -> Json<DaprEventResponse> {
    tracing::Span::current().record("event_id", event.id.as_str());

    info!(
        operation = ?event.operation,
        entity_type = %event.entity_type,
        "Processing task operation via direct DB client"
    );

    let result = match event.operation {
        DbOperation::Create => {
            match serde_json::from_value::<CreateTask>(event.payload.clone()) {
                Ok(input) => service.create_task(input).await.map(|task| {
                    info!(task_id = %task.id, "Task created");
                }),
                Err(e) => {
                    error!(error = %e, "Failed to deserialize CreateTask payload");
                    return Json(DaprEventResponse::drop_message());
                }
            }
        }

        DbOperation::Read => {
            match parse_uuid_from_payload(&event.payload) {
                Some(id) => service.get_task(id).await.map(|task| {
                    info!(task_id = %task.id, title = %task.title, "Task retrieved");
                }),
                None => {
                    error!("Missing or invalid 'id' in Read payload");
                    return Json(DaprEventResponse::drop_message());
                }
            }
        }

        DbOperation::Update => {
            let id = match parse_uuid_from_payload(&event.payload) {
                Some(id) => id,
                None => {
                    error!("Missing or invalid 'id' in Update payload");
                    return Json(DaprEventResponse::drop_message());
                }
            };
            match serde_json::from_value::<UpdateTask>(event.payload.clone()) {
                Ok(input) => service.update_task(id, input).await.map(|task| {
                    info!(task_id = %task.id, "Task updated");
                }),
                Err(e) => {
                    error!(error = %e, "Failed to deserialize UpdateTask payload");
                    return Json(DaprEventResponse::drop_message());
                }
            }
        }

        DbOperation::Delete => {
            match parse_uuid_from_payload(&event.payload) {
                Some(id) => service.delete_task(id).await.map(|_| {
                    info!(task_id = %id, "Task deleted");
                }),
                None => {
                    error!("Missing or invalid 'id' in Delete payload");
                    return Json(DaprEventResponse::drop_message());
                }
            }
        }

        DbOperation::Query => {
            let filter = event
                .payload
                .get("filter")
                .and_then(|f| serde_json::from_value::<TaskFilter>(f.clone()).ok())
                .unwrap_or_default();

            service.list_tasks(filter).await.map(|tasks| {
                info!(count = tasks.len(), "Task query completed");
            })
        }

        other => {
            warn!(operation = ?other, "Unsupported operation for task backend");
            return Json(DaprEventResponse::drop_message());
        }
    };

    match result {
        Ok(()) => {
            info!(event_id = %event.id, "Task operation completed successfully");
            Json(DaprEventResponse::success())
        }
        Err(e) => {
            error!(event_id = %event.id, error = %e, "Task operation failed");
            if event.retry_count < event.max_retries() {
                Json(DaprEventResponse::retry())
            } else {
                error!(event_id = %event.id, "Max retries exceeded, dropping");
                Json(DaprEventResponse::drop_message())
            }
        }
    }
}

/// Extract a UUID from the "id" field of a JSON payload.
fn parse_uuid_from_payload(payload: &serde_json::Value) -> Option<Uuid> {
    payload
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
}

// ── Fallback: generic Dapr state store handler ───────────────

/// Fallback handler for non-task entity types using Dapr state store API.
#[derive(Clone)]
pub struct DaprStateHandler {
    pub state_client: StateClient,
}

impl DaprStateHandler {
    pub fn new(state_client: StateClient) -> Self {
        Self { state_client }
    }
}

/// Handle a generic DB operation via Dapr state store (key-value).
#[instrument(skip(handler, event), fields(event_id))]
pub async fn handle_db_event(
    State(handler): State<DaprStateHandler>,
    Json(event): Json<CloudEvent<DbOpEvent>>,
) -> Json<DaprEventResponse> {
    let db_event = event.data;
    tracing::Span::current().record("event_id", db_event.id.as_str());

    info!(
        operation = ?db_event.operation,
        entity_type = %db_event.entity_type,
        "Processing generic operation via Dapr state store"
    );

    let result = match db_event.operation {
        DbOperation::Create => {
            let key = format!("{}:{}", db_event.entity_type, db_event.id);
            handler.state_client.save(&key, &db_event.payload).await
        }
        DbOperation::Read => {
            let key = format!(
                "{}:{}",
                db_event.entity_type,
                db_event.payload.get("id").and_then(|v| v.as_str()).unwrap_or(&db_event.id)
            );
            match handler.state_client.get::<serde_json::Value>(&key).await {
                Ok(Some(value)) => {
                    info!(key = %key, "State retrieved: {}", value);
                    Ok(())
                }
                Ok(None) => {
                    warn!(key = %key, "State not found");
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        DbOperation::Update => {
            let key = format!(
                "{}:{}",
                db_event.entity_type,
                db_event.payload.get("id").and_then(|v| v.as_str()).unwrap_or(&db_event.id)
            );
            handler.state_client.save(&key, &db_event.payload).await
        }
        DbOperation::Delete => {
            let key = format!(
                "{}:{}",
                db_event.entity_type,
                db_event.payload.get("id").and_then(|v| v.as_str()).unwrap_or(&db_event.id)
            );
            handler.state_client.delete(&key).await
        }
        DbOperation::Query => {
            match handler.state_client.query::<serde_json::Value>(&db_event.payload).await {
                Ok(response) => {
                    info!(results = response.results.len(), "Query completed");
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        other => {
            warn!(operation = ?other, "Unsupported operation for Dapr state store");
            return Json(DaprEventResponse::drop_message());
        }
    };

    match result {
        Ok(()) => Json(DaprEventResponse::success()),
        Err(e) => {
            error!(event_id = %db_event.id, error = %e, "Operation failed");
            if db_event.retry_count < db_event.max_retries() {
                Json(DaprEventResponse::retry())
            } else {
                Json(DaprEventResponse::drop_message())
            }
        }
    }
}
