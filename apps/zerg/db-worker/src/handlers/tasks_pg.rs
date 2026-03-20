//! Handler for PostgreSQL task operations using the domain TaskService directly.

use crate::handlers::dapr_state::dispatch_task_op;
use dapr_client::subscription::{CloudEvent, DaprEventResponse};
use axum::Json;
use axum::extract::State;
use domain_tasks::{PgTaskRepository, TaskService};
use messaging::DbOpEvent;
use std::sync::Arc;
use tracing::instrument;

/// Shared state for PostgreSQL task handlers.
#[derive(Clone)]
pub struct PgTaskHandler {
    pub service: Arc<TaskService<PgTaskRepository>>,
}

impl PgTaskHandler {
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        let repository = PgTaskRepository::new(db);
        let service = TaskService::new(repository);
        Self {
            service: Arc::new(service),
        }
    }
}

/// Handle a task operation event targeting PostgreSQL.
#[instrument(skip_all, fields(event_id))]
pub async fn handle_event(
    State(handler): State<PgTaskHandler>,
    Json(event): Json<CloudEvent<DbOpEvent>>,
) -> Json<DaprEventResponse> {
    dispatch_task_op(&handler.service, event.data).await
}
