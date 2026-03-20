//! Composable data transformation pipelines.
//!
//! A pipeline is a named sequence of transformations applied to a source dataset.
//! Pipelines track their lineage and can be registered back into the catalog.

use crate::catalog::DataCatalog;
use crate::error::{AnalyticsError, AnalyticsResult};
use crate::transform::Transforms;
use polars::prelude::*;

/// A composable data pipeline.
pub struct Pipeline {
    name: String,
    lf: Option<LazyFrame>,
    sources: Vec<String>,
}

impl Pipeline {
    /// Create a new named pipeline.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            lf: None,
            sources: Vec::new(),
        }
    }

    /// Set the source dataset from the catalog.
    pub fn source(mut self, dataset: &str, catalog: &DataCatalog) -> AnalyticsResult<Self> {
        self.lf = Some(catalog.get_lazy(dataset)?);
        self.sources.push(dataset.to_string());
        Ok(self)
    }

    /// Set the source from a LazyFrame directly.
    pub fn from_lazy(mut self, lf: LazyFrame) -> Self {
        self.lf = Some(lf);
        self
    }

    /// Join with another dataset from the catalog.
    pub fn join(
        mut self,
        dataset: &str,
        catalog: &DataCatalog,
        left_on: &str,
        right_on: &str,
    ) -> AnalyticsResult<Self> {
        let right = catalog.get_lazy(dataset)?;
        self.sources.push(dataset.to_string());
        self.lf = Some(
            self.lf
                .ok_or_else(|| AnalyticsError::Pipeline("No source set".into()))?
                .join(right, [col(left_on)], [col(right_on)], JoinArgs::new(JoinType::Left)),
        );
        Ok(self)
    }

    /// Filter to completed orders only.
    pub fn filter_completed(mut self) -> Self {
        self.lf = self.lf.map(|lf| lf.filter_completed());
        self
    }

    /// Add a revenue column (quantity * unit_price).
    pub fn add_revenue_column(mut self) -> Self {
        self.lf = self.lf.map(|lf| lf.add_revenue());
        self
    }

    /// Filter by status.
    pub fn filter_status(mut self, status: &str) -> Self {
        self.lf = self.lf.map(|lf| lf.filter_status(status));
        self
    }

    /// Filter by region.
    pub fn filter_region(mut self, region: &str) -> Self {
        self.lf = self.lf.map(|lf| lf.filter_region(region));
        self
    }

    /// Group by a column and aggregate revenue.
    pub fn revenue_by(mut self, group_col: &str) -> Self {
        self.lf = self.lf.map(|lf| lf.revenue_by(group_col));
        self
    }

    /// Select specific columns.
    pub fn select(mut self, cols: &[&str]) -> Self {
        let exprs: Vec<Expr> = cols.iter().map(|c| col(*c)).collect();
        self.lf = self.lf.map(|lf| lf.select(exprs));
        self
    }

    /// Add a custom expression as a new column.
    pub fn with_column(mut self, expr: Expr) -> Self {
        self.lf = self.lf.map(|lf| lf.with_column(expr));
        self
    }

    /// Apply a custom filter expression.
    pub fn filter(mut self, expr: Expr) -> Self {
        self.lf = self.lf.map(|lf| lf.filter(expr));
        self
    }

    /// Limit to N rows.
    pub fn limit(mut self, n: u32) -> Self {
        self.lf = self.lf.map(|lf| lf.limit(n));
        self
    }

    /// Sort by columns.
    pub fn sort(mut self, cols: &[&str], descending: bool) -> Self {
        let sort_cols: Vec<PlSmallStr> = cols.iter().map(|c| PlSmallStr::from(*c)).collect();
        self.lf = self.lf.map(|lf| {
            lf.sort(sort_cols, SortMultipleOptions::default().with_order_descending(descending))
        });
        self
    }

    /// Execute the pipeline and return the result.
    pub fn execute(self) -> AnalyticsResult<DataFrame> {
        self.lf
            .ok_or_else(|| AnalyticsError::Pipeline("No source set".into()))?
            .collect()
            .map_err(AnalyticsError::from)
    }

    /// Execute and register the result in the catalog.
    pub fn execute_into(self, catalog: &mut DataCatalog, output_name: &str) -> AnalyticsResult<DataFrame> {
        let name = self.name.clone();
        let sources = self.sources.clone();
        let df = self.execute()?;
        catalog.register_transform(output_name, df.clone(), &name, sources);
        Ok(df)
    }

    /// Get the pipeline name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> DataCatalog {
        let mut c = DataCatalog::new();
        c.register_csv("sales", "data/sales.csv").unwrap();
        c.register_csv("customers", "data/customers.csv").unwrap();
        c
    }

    #[test]
    fn test_basic_pipeline() {
        let cat = catalog();
        let df = Pipeline::new("test")
            .source("sales", &cat)
            .unwrap()
            .filter_completed()
            .add_revenue_column()
            .execute()
            .unwrap();

        assert!(df.height() > 0);
        assert!(df.get_column_names().contains(&&PlSmallStr::from("revenue")));
    }

    #[test]
    fn test_revenue_by_category() {
        let cat = catalog();
        let df = Pipeline::new("category_revenue")
            .source("sales", &cat)
            .unwrap()
            .filter_completed()
            .add_revenue_column()
            .revenue_by("category")
            .execute()
            .unwrap();

        assert_eq!(df.height(), 2); // electronics, furniture
        let categories = df.column("category").unwrap();
        let cat_strs: Vec<&str> = categories.str().unwrap().into_no_null_iter().collect();
        assert!(cat_strs.contains(&"electronics"));
        assert!(cat_strs.contains(&"furniture"));
    }

    #[test]
    fn test_pipeline_with_join() {
        let cat = catalog();
        let df = Pipeline::new("enriched_sales")
            .source("sales", &cat)
            .unwrap()
            .join("customers", &cat, "customer_id", "customer_id")
            .unwrap()
            .filter_completed()
            .add_revenue_column()
            .select(&["order_id", "name", "product", "revenue", "tier"])
            .execute()
            .unwrap();

        assert!(df.height() > 0);
        assert!(df.get_column_names().contains(&&PlSmallStr::from("name")));
        assert!(df.get_column_names().contains(&&PlSmallStr::from("tier")));
    }

    #[test]
    fn test_execute_into_catalog() {
        let mut cat = catalog();
        let df = Pipeline::new("top_products")
            .source("sales", &cat)
            .unwrap()
            .filter_completed()
            .add_revenue_column()
            .sort(&["revenue"], true)
            .limit(5)
            .execute_into(&mut cat, "top_5_products")
            .unwrap();

        assert_eq!(df.height(), 5);
        assert!(cat.meta("top_5_products").is_some());

        let lineage = cat.lineage("top_5_products");
        assert!(lineage.iter().any(|l| l.contains("top_products")));
    }

    #[test]
    fn test_region_filter_pipeline() {
        let cat = catalog();
        let df = Pipeline::new("north_revenue")
            .source("sales", &cat)
            .unwrap()
            .filter_completed()
            .filter_region("north")
            .add_revenue_column()
            .revenue_by("category")
            .execute()
            .unwrap();

        assert!(df.height() > 0);
    }

    #[test]
    fn test_custom_filter_and_column() {
        let cat = catalog();
        let df = Pipeline::new("high_value")
            .source("sales", &cat)
            .unwrap()
            .add_revenue_column()
            .filter(col("revenue").gt(lit(500.0)))
            .with_column((col("revenue") * lit(0.1)).alias("commission"))
            .execute()
            .unwrap();

        assert!(df.height() > 0);
        assert!(df.get_column_names().contains(&&PlSmallStr::from("commission")));
    }
}
