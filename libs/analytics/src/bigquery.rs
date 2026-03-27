//! BigQuery export for analytics data.
//!
//! Writes DataFrames and DORA metrics to Google BigQuery tables.
//!
//! # Authentication
//!
//! Uses the standard GCP credential chain:
//! - `GOOGLE_APPLICATION_CREDENTIALS` env var pointing to a service account JSON
//! - Workload Identity on GKE
//! - Application Default Credentials (`gcloud auth application-default login`)
//!
//! # Required env vars
//!
//! - `BQ_PROJECT_ID` — GCP project ID
//! - `BQ_DATASET_ID` — BigQuery dataset name (default: `dora_metrics`)
//!
//! # Usage
//!
//! ```rust,ignore
//! use analytics::bigquery::BigQueryExporter;
//! use analytics::{DataCatalog, dora};
//!
//! let exporter = BigQueryExporter::from_env().await?;
//! let catalog = load_dora_data();
//! exporter.export_dora_summary(&catalog).await?;
//! exporter.export_deployments(&catalog).await?;
//! ```

use crate::catalog::DataCatalog;
use crate::dora::{self, DoraMetrics};
use crate::error::{AnalyticsError, AnalyticsResult};

use gcp_bigquery_client::model::table_data_insert_all_request::TableDataInsertAllRequest;
use gcp_bigquery_client::Client;
use polars::prelude::*;
use serde::Serialize;
use tracing::{debug, info};

/// BigQuery table names for DORA data.
pub mod tables {
    pub const DEPLOYMENTS: &str = "deployments";
    pub const INCIDENTS: &str = "incidents";
    pub const PULL_REQUESTS: &str = "pull_requests";
    pub const DORA_SUMMARY: &str = "dora_summary";
    pub const DEPLOYMENT_FREQUENCY: &str = "deployment_frequency";
    pub const LEAD_TIME: &str = "lead_time";
    pub const CHANGE_FAILURE_RATE: &str = "change_failure_rate";
    pub const MTTR: &str = "mttr";
}

/// BigQuery exporter for analytics data.
pub struct BigQueryExporter {
    client: Client,
    project_id: String,
    dataset_id: String,
}

impl BigQueryExporter {
    /// Create from a pre-built client.
    pub fn new(client: Client, project_id: String, dataset_id: String) -> Self {
        Self {
            client,
            project_id,
            dataset_id,
        }
    }

    /// Create from environment variables.
    ///
    /// Expects:
    /// - `GOOGLE_APPLICATION_CREDENTIALS` or default credentials
    /// - `BQ_PROJECT_ID`
    /// - `BQ_DATASET_ID` (optional, defaults to `dora_metrics`)
    pub async fn from_env() -> AnalyticsResult<Self> {
        let project_id = std::env::var("BQ_PROJECT_ID")
            .map_err(|_| AnalyticsError::Pipeline("BQ_PROJECT_ID not set".into()))?;

        let dataset_id =
            std::env::var("BQ_DATASET_ID").unwrap_or_else(|_| "dora_metrics".to_string());

        let sa_key_path = std::env::var("GOOGLE_APPLICATION_CREDENTIALS").ok();

        let client = if let Some(path) = sa_key_path {
            let sa_key = gcp_bigquery_client::yup_oauth2::read_service_account_key(&path)
                .await
                .map_err(|e| AnalyticsError::Pipeline(format!("Failed to read SA key: {}", e)))?;
            Client::from_service_account_key(sa_key, false)
                .await
                .map_err(|e| {
                    AnalyticsError::Pipeline(format!("Failed to create BQ client: {}", e))
                })?
        } else {
            Client::from_application_default_credentials()
                .await
                .map_err(|e| {
                    AnalyticsError::Pipeline(format!(
                        "Failed to create BQ client from ADC: {}",
                        e
                    ))
                })?
        };

        info!(
            project_id = %project_id,
            dataset_id = %dataset_id,
            "BigQuery exporter initialized"
        );

        Ok(Self {
            client,
            project_id,
            dataset_id,
        })
    }

    /// Export a DataFrame to a BigQuery table via streaming insert.
    pub async fn export_dataframe(
        &self,
        table_name: &str,
        df: &DataFrame,
    ) -> AnalyticsResult<usize> {
        let rows = dataframe_to_rows(df)?;
        let row_count = rows.len();

        if rows.is_empty() {
            debug!(table = table_name, "No rows to insert");
            return Ok(0);
        }

        let mut request = TableDataInsertAllRequest::new();
        for row in &rows {
            request.add_row(None, row).map_err(|e| {
                AnalyticsError::Pipeline(format!("Failed to serialize row: {}", e))
            })?;
        }

        self.client
            .tabledata()
            .insert_all(&self.project_id, &self.dataset_id, table_name, request)
            .await
            .map_err(|e| AnalyticsError::Pipeline(format!("BigQuery insert failed: {}", e)))?;

        info!(
            table = table_name,
            rows = row_count,
            "Exported to BigQuery"
        );

        Ok(row_count)
    }

    /// Export raw deployment events from the catalog.
    pub async fn export_deployments(&self, catalog: &DataCatalog) -> AnalyticsResult<usize> {
        let df = catalog.get("deployments")?;
        self.export_dataframe(tables::DEPLOYMENTS, df).await
    }

    /// Export raw incident events from the catalog.
    pub async fn export_incidents(&self, catalog: &DataCatalog) -> AnalyticsResult<usize> {
        let df = catalog.get("incidents")?;
        self.export_dataframe(tables::INCIDENTS, df).await
    }

    /// Export raw pull request data from the catalog.
    pub async fn export_pull_requests(&self, catalog: &DataCatalog) -> AnalyticsResult<usize> {
        let df = catalog.get("pull_requests")?;
        self.export_dataframe(tables::PULL_REQUESTS, df).await
    }

    /// Compute and export the DORA summary as a single row.
    pub async fn export_dora_summary(&self, catalog: &DataCatalog) -> AnalyticsResult<usize> {
        let summary = dora::compute_dora_summary(catalog)?;
        let row = DoraSummaryRow::from(&summary);

        let mut request = TableDataInsertAllRequest::new();
        request.add_row(None, &row).map_err(|e| {
            AnalyticsError::Pipeline(format!("Failed to serialize DORA summary: {}", e))
        })?;

        self.client
            .tabledata()
            .insert_all(
                &self.project_id,
                &self.dataset_id,
                tables::DORA_SUMMARY,
                request,
            )
            .await
            .map_err(|e| AnalyticsError::Pipeline(format!("BigQuery insert failed: {}", e)))?;

        info!("Exported DORA summary to BigQuery");
        Ok(1)
    }

    /// Export all computed DORA metric breakdowns.
    pub async fn export_all_dora_metrics(
        &self,
        catalog: &DataCatalog,
    ) -> AnalyticsResult<usize> {
        let mut total = 0;

        // Summary
        total += self.export_dora_summary(catalog).await?;

        // Deployment frequency by service
        let df = dora::deployment_frequency_by_service(catalog)?;
        total += self
            .export_dataframe(tables::DEPLOYMENT_FREQUENCY, &df)
            .await?;

        // Lead time by team
        let df = dora::lead_time_by_team(catalog)?;
        total += self.export_dataframe(tables::LEAD_TIME, &df).await?;

        // Change failure rate by service
        let df = dora::change_failure_rate_by_service(catalog)?;
        total += self
            .export_dataframe(tables::CHANGE_FAILURE_RATE, &df)
            .await?;

        // MTTR by severity
        let df = dora::mttr_by_severity(catalog)?;
        total += self.export_dataframe(tables::MTTR, &df).await?;

        info!(total_rows = total, "Exported all DORA metrics to BigQuery");
        Ok(total)
    }

    /// Export raw event data (deployments, incidents, PRs).
    pub async fn export_all_raw_data(&self, catalog: &DataCatalog) -> AnalyticsResult<usize> {
        let mut total = 0;
        total += self.export_deployments(catalog).await?;
        total += self.export_incidents(catalog).await?;
        total += self.export_pull_requests(catalog).await?;

        info!(
            total_rows = total,
            "Exported all raw DORA data to BigQuery"
        );
        Ok(total)
    }
}

// =============================================================================
// Serialization helpers
// =============================================================================

/// DORA summary row for BigQuery.
#[derive(Debug, Serialize)]
struct DoraSummaryRow {
    computed_at: String,
    deployment_frequency_per_day: f64,
    deployment_frequency_level: String,
    lead_time_hours: f64,
    lead_time_level: String,
    change_failure_rate: f64,
    change_failure_rate_level: String,
    mttr_hours: f64,
    mttr_level: String,
}

impl From<&DoraMetrics> for DoraSummaryRow {
    fn from(m: &DoraMetrics) -> Self {
        Self {
            computed_at: chrono::Utc::now().to_rfc3339(),
            deployment_frequency_per_day: m.deployment_frequency_per_day,
            deployment_frequency_level: m.deployment_frequency_level.to_string(),
            lead_time_hours: m.lead_time_hours,
            lead_time_level: m.lead_time_level.to_string(),
            change_failure_rate: m.change_failure_rate,
            change_failure_rate_level: m.change_failure_rate_level.to_string(),
            mttr_hours: m.mttr_hours,
            mttr_level: m.mttr_level.to_string(),
        }
    }
}

/// Convert a Polars DataFrame to a Vec of serde_json::Value rows for BQ streaming insert.
fn dataframe_to_rows(df: &DataFrame) -> AnalyticsResult<Vec<serde_json::Value>> {
    let columns = df.get_columns();
    let col_names: Vec<&str> = columns
        .iter()
        .map(|c| c.name().as_str())
        .collect();

    let mut rows = Vec::with_capacity(df.height());

    for row_idx in 0..df.height() {
        let mut map = serde_json::Map::new();
        for (col_idx, name) in col_names.iter().enumerate() {
            let col = &columns[col_idx];
            let series = col.as_materialized_series();
            let value = series_value_at(series, row_idx);
            map.insert(name.to_string(), value);
        }
        rows.push(serde_json::Value::Object(map));
    }

    Ok(rows)
}

/// Extract a single value from a Series at a given index as serde_json::Value.
fn series_value_at(series: &Series, idx: usize) -> serde_json::Value {
    let nulls = series.is_null();
    if nulls.get(idx).unwrap_or(false) {
        return serde_json::Value::Null;
    }

    match series.dtype() {
        DataType::Boolean => {
            let val = series.bool().unwrap().get(idx).unwrap();
            serde_json::Value::Bool(val)
        }
        DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            let val = series.cast(&DataType::UInt64).unwrap();
            let v = val.u64().unwrap().get(idx).unwrap();
            serde_json::json!(v)
        }
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => {
            let val = series.cast(&DataType::Int64).unwrap();
            let v = val.i64().unwrap().get(idx).unwrap();
            serde_json::json!(v)
        }
        DataType::Float32 | DataType::Float64 => {
            let val = series.cast(&DataType::Float64).unwrap();
            let v = val.f64().unwrap().get(idx).unwrap();
            serde_json::json!(v)
        }
        DataType::String => {
            let val = series.str().unwrap().get(idx).unwrap_or("");
            serde_json::Value::String(val.to_string())
        }
        _ => {
            // Fallback: cast to string
            let val = series.cast(&DataType::String).unwrap();
            let v = val.str().unwrap().get(idx).unwrap_or("");
            serde_json::Value::String(v.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dataframe_to_rows() {
        let df = df!(
            "name" => ["api", "web"],
            "count" => [10u32, 5],
            "rate" => [0.25, 0.1],
            "active" => [true, false],
        )
        .unwrap();

        let rows = dataframe_to_rows(&df).unwrap();
        assert_eq!(rows.len(), 2);

        let first = &rows[0];
        assert_eq!(first["name"], "api");
        assert_eq!(first["count"], 10);
        assert_eq!(first["rate"], 0.25);
        assert_eq!(first["active"], true);
    }

    #[test]
    fn test_dora_summary_row_serialization() {
        let metrics = DoraMetrics {
            deployment_frequency_per_day: 0.95,
            deployment_frequency_level: crate::dora::DoraLevel::High,
            lead_time_hours: 3.8,
            lead_time_level: crate::dora::DoraLevel::Elite,
            change_failure_rate: 0.222,
            change_failure_rate_level: crate::dora::DoraLevel::Low,
            mttr_hours: 2.5,
            mttr_level: crate::dora::DoraLevel::High,
        };

        let row = DoraSummaryRow::from(&metrics);
        let json = serde_json::to_value(&row).unwrap();

        assert_eq!(json["deployment_frequency_level"], "High");
        assert_eq!(json["lead_time_level"], "Elite");
        assert_eq!(json["change_failure_rate_level"], "Low");
        assert_eq!(json["mttr_level"], "High");
    }

    #[test]
    fn test_series_value_at_types() {
        let s_str = Series::new("a".into(), &["hello"]);
        assert_eq!(series_value_at(&s_str, 0), serde_json::json!("hello"));

        let s_int = Series::new("b".into(), &[42i64]);
        assert_eq!(series_value_at(&s_int, 0), serde_json::json!(42));

        let s_float = Series::new("c".into(), &[3.14f64]);
        assert_eq!(series_value_at(&s_float, 0), serde_json::json!(3.14));

        let s_bool = Series::new("d".into(), &[true]);
        assert_eq!(series_value_at(&s_bool, 0), serde_json::json!(true));
    }
}
