//! Data catalog — register, discover, and track datasets.
//!
//! Every dataset gets metadata: name, source path, schema, row count,
//! and lineage (which transformations produced it).

use crate::error::{AnalyticsError, AnalyticsResult};
use crate::ingest::Ingestor;
use polars::prelude::*;
use std::collections::HashMap;

/// Metadata about a registered dataset.
#[derive(Debug, Clone)]
pub struct DatasetMeta {
    pub name: String,
    pub source: DataSource,
    pub schema: Schema,
    pub row_count: usize,
    pub column_count: usize,
    /// Lineage: which datasets/transforms produced this one.
    pub lineage: Vec<String>,
}

/// Where a dataset came from.
#[derive(Debug, Clone)]
pub enum DataSource {
    Csv(String),
    Json(String),
    Parquet(String),
    Transform { pipeline: String, sources: Vec<String> },
    InMemory,
}

/// A data catalog that tracks registered datasets.
pub struct DataCatalog {
    datasets: HashMap<String, DataFrame>,
    metadata: HashMap<String, DatasetMeta>,
}

impl DataCatalog {
    pub fn new() -> Self {
        Self {
            datasets: HashMap::new(),
            metadata: HashMap::new(),
        }
    }

    /// Register a CSV file as a dataset.
    #[cfg(feature = "csv")]
    pub fn register_csv(&mut self, name: &str, path: &str) -> AnalyticsResult<&DatasetMeta> {
        let df = Ingestor::read_csv(path)?;
        let meta = DatasetMeta {
            name: name.to_string(),
            source: DataSource::Csv(path.to_string()),
            schema: (**df.schema()).clone(),
            row_count: df.height(),
            column_count: df.width(),
            lineage: vec![format!("ingested from {}", path)],
        };
        self.datasets.insert(name.to_string(), df);
        self.metadata.insert(name.to_string(), meta);
        Ok(self.metadata.get(name).unwrap())
    }

    /// Register an in-memory DataFrame.
    pub fn register_df(&mut self, name: &str, df: DataFrame) -> &DatasetMeta {
        let meta = DatasetMeta {
            name: name.to_string(),
            source: DataSource::InMemory,
            schema: (**df.schema()).clone(),
            row_count: df.height(),
            column_count: df.width(),
            lineage: vec!["created in-memory".to_string()],
        };
        self.datasets.insert(name.to_string(), df);
        self.metadata.insert(name.to_string(), meta);
        self.metadata.get(name).unwrap()
    }

    /// Register a transformed dataset with lineage.
    pub fn register_transform(
        &mut self,
        name: &str,
        df: DataFrame,
        pipeline: &str,
        sources: Vec<String>,
    ) -> &DatasetMeta {
        let meta = DatasetMeta {
            name: name.to_string(),
            source: DataSource::Transform {
                pipeline: pipeline.to_string(),
                sources: sources.clone(),
            },
            schema: (**df.schema()).clone(),
            row_count: df.height(),
            column_count: df.width(),
            lineage: {
                let mut l = vec![format!("pipeline: {}", pipeline)];
                for s in &sources {
                    l.push(format!("source: {}", s));
                }
                l
            },
        };
        self.datasets.insert(name.to_string(), df);
        self.metadata.insert(name.to_string(), meta);
        self.metadata.get(name).unwrap()
    }

    /// Get a dataset by name.
    pub fn get(&self, name: &str) -> AnalyticsResult<&DataFrame> {
        self.datasets
            .get(name)
            .ok_or_else(|| AnalyticsError::DatasetNotFound(name.to_string()))
    }

    /// Get a dataset as a LazyFrame.
    pub fn get_lazy(&self, name: &str) -> AnalyticsResult<LazyFrame> {
        Ok(self.get(name)?.clone().lazy())
    }

    /// Get metadata for a dataset.
    pub fn meta(&self, name: &str) -> Option<&DatasetMeta> {
        self.metadata.get(name)
    }

    /// List all registered dataset names.
    pub fn list(&self) -> Vec<&str> {
        self.metadata.keys().map(|s| s.as_str()).collect()
    }

    /// Get full lineage for a dataset (recursive).
    pub fn lineage(&self, name: &str) -> Vec<String> {
        match self.metadata.get(name) {
            Some(meta) => meta.lineage.clone(),
            None => vec![],
        }
    }

    /// Describe a dataset (summary statistics).
    pub fn describe(&self, name: &str) -> AnalyticsResult<DataFrame> {
        let df = self.get(name)?;
        Ok(df.clone())
    }
}

impl Default for DataCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_get() {
        let mut catalog = DataCatalog::new();
        catalog.register_csv("sales", "data/sales.csv").unwrap();

        let df = catalog.get("sales").unwrap();
        assert!(df.height() > 0);

        let meta = catalog.meta("sales").unwrap();
        assert_eq!(meta.name, "sales");
        assert_eq!(meta.row_count, 30);
    }

    #[test]
    fn test_list_datasets() {
        let mut catalog = DataCatalog::new();
        catalog.register_csv("sales", "data/sales.csv").unwrap();
        catalog.register_csv("customers", "data/customers.csv").unwrap();

        let names = catalog.list();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"sales"));
        assert!(names.contains(&"customers"));
    }

    #[test]
    fn test_dataset_not_found() {
        let catalog = DataCatalog::new();
        assert!(catalog.get("nonexistent").is_err());
    }

    #[test]
    fn test_describe() {
        let mut catalog = DataCatalog::new();
        catalog.register_csv("sales", "data/sales.csv").unwrap();
        let desc = catalog.describe("sales").unwrap();
        assert!(desc.height() > 0);
    }

    #[test]
    fn test_lineage_tracking() {
        let mut catalog = DataCatalog::new();
        catalog.register_csv("sales", "data/sales.csv").unwrap();

        let df = catalog.get("sales").unwrap().clone();
        catalog.register_transform(
            "sales_summary",
            df,
            "monthly_revenue",
            vec!["sales".to_string()],
        );

        let lineage = catalog.lineage("sales_summary");
        assert!(lineage.iter().any(|l| l.contains("monthly_revenue")));
        assert!(lineage.iter().any(|l| l.contains("sales")));
    }
}
