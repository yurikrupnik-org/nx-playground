//! Handler for InfluxDB time-series operations.

use dapr_client::subscription::{CloudEvent, DaprEventResponse};
use axum::extract::State;
use axum::Json;
use influxdb2::Client;
use messaging::DbOpEvent;
use messaging::jobs::DbOperation;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

/// Shared state for InfluxDB handlers.
#[derive(Clone)]
pub struct InfluxDbHandler {
    pub client: Arc<Client>,
    pub bucket: String,
    pub org: String,
}

impl InfluxDbHandler {
    pub fn new(client: Client, bucket: String, org: String) -> Self {
        Self {
            client: Arc::new(client),
            bucket,
            org,
        }
    }
}

/// Handle time-series operation events.
#[instrument(skip(handler, event), fields(event_id))]
pub async fn handle_timeseries_event(
    State(handler): State<InfluxDbHandler>,
    Json(event): Json<CloudEvent<DbOpEvent>>,
) -> Json<DaprEventResponse> {
    let db_event = event.data;
    tracing::Span::current().record("event_id", &db_event.id.as_str());

    info!(
        operation = ?db_event.operation,
        entity_type = %db_event.entity_type,
        "Processing time-series operation"
    );

    let result = match db_event.operation {
        DbOperation::TimeSeriesWrite => {
            handle_ts_write(&handler, &db_event).await
        }
        DbOperation::TimeSeriesQuery => {
            handle_ts_query(&handler, &db_event).await
        }
        other => {
            warn!(operation = ?other, "Unsupported operation for InfluxDB backend");
            return Json(DaprEventResponse::drop_message());
        }
    };

    match result {
        Ok(()) => {
            info!(event_id = %db_event.id, "Time-series operation completed");
            Json(DaprEventResponse::success())
        }
        Err(e) => {
            error!(event_id = %db_event.id, error = %e, "Time-series operation failed");
            if db_event.retry_count < db_event.max_retries() {
                Json(DaprEventResponse::retry())
            } else {
                Json(DaprEventResponse::drop_message())
            }
        }
    }
}

async fn handle_ts_write(handler: &InfluxDbHandler, event: &DbOpEvent) -> eyre::Result<()> {
    let line_protocol = event
        .payload
        .get("line_protocol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| eyre::eyre!("Missing 'line_protocol' field in payload"))?;

    handler
        .client
        .write_line_protocol(&handler.org, &handler.bucket, line_protocol)
        .await?;

    info!(
        measurement = %event.entity_type,
        "Time-series data written"
    );
    Ok(())
}

async fn handle_ts_query(handler: &InfluxDbHandler, event: &DbOpEvent) -> eyre::Result<()> {
    let flux_query = event
        .payload
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| eyre::eyre!("Missing 'query' field in payload"))?;

    let results: Vec<serde_json::Value> = handler
        .client
        .query(Some(influxdb2::models::Query::new(flux_query.to_string())), &handler.org)
        .await?;

    info!(
        results = results.len(),
        "Time-series query completed"
    );
    Ok(())
}
