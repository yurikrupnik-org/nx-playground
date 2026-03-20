//! Handler for Neo4j graph database operations.

use dapr_client::subscription::{CloudEvent, DaprEventResponse};
use axum::extract::State;
use axum::Json;
use messaging::DbOpEvent;
use messaging::jobs::DbOperation;
use neo4rs::Graph;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

/// Shared state for Neo4j handlers.
#[derive(Clone)]
pub struct Neo4jHandler {
    pub graph: Arc<Graph>,
}

impl Neo4jHandler {
    pub fn new(graph: Graph) -> Self {
        Self {
            graph: Arc::new(graph),
        }
    }
}

/// Handle graph operation events.
#[instrument(skip(handler, event), fields(event_id))]
pub async fn handle_graph_event(
    State(handler): State<Neo4jHandler>,
    Json(event): Json<CloudEvent<DbOpEvent>>,
) -> Json<DaprEventResponse> {
    let db_event = event.data;
    tracing::Span::current().record("event_id", &db_event.id.as_str());

    info!(
        operation = ?db_event.operation,
        entity_type = %db_event.entity_type,
        "Processing graph operation"
    );

    let result = match db_event.operation {
        DbOperation::GraphTraverse => {
            handle_graph_traverse(&handler, &db_event).await
        }
        DbOperation::GraphMutate => {
            handle_graph_mutate(&handler, &db_event).await
        }
        other => {
            warn!(operation = ?other, "Unsupported operation for Neo4j backend");
            return Json(DaprEventResponse::drop_message());
        }
    };

    match result {
        Ok(()) => {
            info!(event_id = %db_event.id, "Graph operation completed");
            Json(DaprEventResponse::success())
        }
        Err(e) => {
            error!(event_id = %db_event.id, error = %e, "Graph operation failed");
            if db_event.retry_count < db_event.max_retries() {
                Json(DaprEventResponse::retry())
            } else {
                Json(DaprEventResponse::drop_message())
            }
        }
    }
}

async fn handle_graph_traverse(handler: &Neo4jHandler, event: &DbOpEvent) -> eyre::Result<()> {
    let cypher = event
        .payload
        .get("cypher")
        .and_then(|v| v.as_str())
        .ok_or_else(|| eyre::eyre!("Missing 'cypher' field in payload"))?;

    let mut result = handler.graph.execute(neo4rs::query(cypher)).await?;

    let mut count = 0u64;
    while let Some(_row) = result.next().await? {
        count += 1;
    }

    info!(
        rows = count,
        "Graph traversal completed"
    );
    Ok(())
}

async fn handle_graph_mutate(handler: &Neo4jHandler, event: &DbOpEvent) -> eyre::Result<()> {
    let cypher = event
        .payload
        .get("cypher")
        .and_then(|v| v.as_str())
        .ok_or_else(|| eyre::eyre!("Missing 'cypher' field in payload"))?;

    handler.graph.run(neo4rs::query(cypher)).await?;

    info!(
        entity_type = %event.entity_type,
        "Graph mutation completed"
    );
    Ok(())
}
