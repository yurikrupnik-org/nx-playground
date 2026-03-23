//! Analytics engine powered by Polars.
//!
//! A mini data platform (inspired by Matia.io) providing:
//! - **Ingestion**: Load data from CSV, JSON, Parquet files
//! - **Transformations**: Clean, join, aggregate, pivot data
//! - **Pipeline**: Composable transformation steps
//! - **Catalog**: Track datasets with metadata and lineage
//! - **Export**: Write results to any supported format
//!
//! # Example
//!
//! ```rust,no_run
//! use analytics::{Pipeline, DataCatalog};
//!
//! let mut catalog = DataCatalog::new();
//! catalog.register_csv("sales", "data/sales.csv").unwrap();
//!
//! let result = Pipeline::new("category_revenue")
//!     .source("sales", &catalog)
//!     .unwrap()
//!     .filter_completed()
//!     .add_revenue_column()
//!     .revenue_by("category")
//!     .execute()
//!     .unwrap();
//!
//! println!("{}", result);
//! ```

pub mod catalog;
pub mod dora;
pub mod error;
pub mod ingest;
pub mod pipeline;
pub mod transform;

pub use catalog::DataCatalog;
pub use dora::{compute_dora_summary, DoraLevel, DoraMetrics};
pub use error::{AnalyticsError, AnalyticsResult};
pub use ingest::Ingestor;
pub use pipeline::Pipeline;
pub use transform::Transforms;
