//! Error types for the analytics engine.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnalyticsError {
    #[error("Polars error: {0}")]
    Polars(#[from] polars::prelude::PolarsError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Dataset not found: {0}")]
    DatasetNotFound(String),

    #[error("Column not found: {0}")]
    ColumnNotFound(String),

    #[error("Pipeline error: {0}")]
    Pipeline(String),

    #[error("Ingestion error: {0}")]
    Ingestion(String),
}

pub type AnalyticsResult<T> = Result<T, AnalyticsError>;
