//! Handler for MongoDB task operations using the domain TaskService directly.

use crate::handlers::dapr_state::dispatch_task_op;
use dapr_client::subscription::{CloudEvent, DaprEventResponse};
use axum::Json;
use axum::extract::State;
use domain_tasks::{MongoTaskRepository, TaskService};
use messaging::DbOpEvent;
use std::sync::Arc;
use tracing::instrument;

/// Shared state for MongoDB task handlers.
#[derive(Clone)]
pub struct MongoTaskHandler {
    pub service: Arc<TaskService<MongoTaskRepository>>,
}

impl MongoTaskHandler {
    pub fn new(db: mongodb::Database) -> Self {
        let repository = MongoTaskRepository::new(db);
        let service = TaskService::new(repository);
        Self {
            service: Arc::new(service),
        }
    }
}

/// Handle a task operation event targeting MongoDB.
#[instrument(skip_all, fields(event_id))]
pub async fn handle_event(
    State(handler): State<MongoTaskHandler>,
    Json(event): Json<CloudEvent<DbOpEvent>>,
) -> Json<DaprEventResponse> {
    dispatch_task_op(&handler.service, event.data).await
}
