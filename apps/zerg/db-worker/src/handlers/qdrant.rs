//! Handler for Qdrant vector database operations.

use dapr_client::subscription::{CloudEvent, DaprEventResponse};
use axum::extract::State;
use axum::Json;
use messaging::DbOpEvent;
use messaging::jobs::DbOperation;
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{PointStruct, SearchPoints, UpsertPointsBuilder};
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

/// Shared state for Qdrant handlers.
#[derive(Clone)]
pub struct QdrantHandler {
    pub client: Arc<Qdrant>,
}

impl QdrantHandler {
    pub fn new(client: Qdrant) -> Self {
        Self {
            client: Arc::new(client),
        }
    }
}

/// Handle vector operation events.
#[instrument(skip(handler, event), fields(event_id))]
pub async fn handle_vector_event(
    State(handler): State<QdrantHandler>,
    Json(event): Json<CloudEvent<DbOpEvent>>,
) -> Json<DaprEventResponse> {
    let db_event = event.data;
    tracing::Span::current().record("event_id", &db_event.id.as_str());

    info!(
        operation = ?db_event.operation,
        entity_type = %db_event.entity_type,
        "Processing vector operation"
    );

    let result = match db_event.operation {
        DbOperation::VectorUpsert => {
            handle_vector_upsert(&handler, &db_event).await
        }
        DbOperation::VectorSearch => {
            handle_vector_search(&handler, &db_event).await
        }
        other => {
            warn!(operation = ?other, "Unsupported operation for Qdrant backend");
            return Json(DaprEventResponse::drop_message());
        }
    };

    match result {
        Ok(()) => {
            info!(event_id = %db_event.id, "Vector operation completed");
            Json(DaprEventResponse::success())
        }
        Err(e) => {
            error!(event_id = %db_event.id, error = %e, "Vector operation failed");
            if db_event.retry_count < db_event.max_retries() {
                Json(DaprEventResponse::retry())
            } else {
                Json(DaprEventResponse::drop_message())
            }
        }
    }
}

async fn handle_vector_upsert(handler: &QdrantHandler, event: &DbOpEvent) -> eyre::Result<()> {
    let collection = event.entity_type.clone();
    let id = event.payload.get("id").and_then(|v| v.as_str()).unwrap_or(&event.id);
    let vector: Vec<f32> = serde_json::from_value(
        event
            .payload
            .get("vector")
            .cloned()
            .ok_or_else(|| eyre::eyre!("Missing 'vector' field in payload"))?,
    )?;

    let payload_data = event
        .payload
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

    let point = PointStruct::new(
        id.to_string(),
        vector,
        serde_json::from_value::<qdrant_client::qdrant::value::Kind>(payload_data)
            .map(|_| std::collections::HashMap::new())
            .unwrap_or_default(),
    );

    handler
        .client
        .upsert_points(UpsertPointsBuilder::new(&collection, vec![point]))
        .await?;

    info!(collection = %collection, id = %id, "Vector upserted");
    Ok(())
}

async fn handle_vector_search(handler: &QdrantHandler, event: &DbOpEvent) -> eyre::Result<()> {
    let collection = event.entity_type.clone();
    let vector: Vec<f32> = serde_json::from_value(
        event
            .payload
            .get("vector")
            .cloned()
            .ok_or_else(|| eyre::eyre!("Missing 'vector' field in payload"))?,
    )?;
    let limit = event
        .payload
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as u64;

    let results = handler
        .client
        .search_points(SearchPoints {
            collection_name: collection.clone(),
            vector,
            limit,
            ..Default::default()
        })
        .await?;

    info!(
        collection = %collection,
        results = results.result.len(),
        "Vector search completed"
    );
    Ok(())
}
