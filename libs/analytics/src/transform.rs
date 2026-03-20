//! Reusable data transformation functions.

use polars::prelude::*;

/// Extension trait for common DataFrame transformations.
pub trait Transforms {
    /// Add a revenue column (quantity * unit_price).
    fn add_revenue(self) -> Self;

    /// Filter to only completed orders.
    fn filter_completed(self) -> Self;

    /// Filter to a specific status.
    fn filter_status(self, status: &str) -> Self;

    /// Filter to a specific region.
    fn filter_region(self, region: &str) -> Self;

    /// Filter to a date range (inclusive).
    fn filter_date_range(self, col: &str, start: &str, end: &str) -> Self;

    /// Aggregate revenue by a grouping column.
    fn revenue_by(self, group_col: &str) -> Self;

    /// Top N rows by a column (descending).
    fn top_n(self, col: &str, n: u32) -> Self;

    /// Add a running total column.
    fn running_total(self, col: &str, alias: &str) -> Self;
}

impl Transforms for LazyFrame {
    fn add_revenue(self) -> Self {
        self.with_column(
            (col("quantity").cast(DataType::Float64) * col("unit_price"))
                .alias("revenue"),
        )
    }

    fn filter_completed(self) -> Self {
        self.filter(col("status").eq(lit("completed")))
    }

    fn filter_status(self, status: &str) -> Self {
        self.filter(col("status").eq(lit(status)))
    }

    fn filter_region(self, region: &str) -> Self {
        self.filter(col("region").eq(lit(region)))
    }

    fn filter_date_range(self, date_col: &str, start: &str, end: &str) -> Self {
        self.filter(
            col(date_col)
                .gt_eq(lit(start))
                .and(col(date_col).lt_eq(lit(end))),
        )
    }

    fn revenue_by(self, group_col: &str) -> Self {
        self.group_by([col(group_col)])
            .agg([
                col("revenue").sum().alias("total_revenue"),
                col("revenue").mean().alias("avg_revenue"),
                col("order_id").count().alias("order_count"),
                col("quantity").sum().alias("total_units"),
            ])
            .sort(["total_revenue"], SortMultipleOptions::default().with_order_descending(true))
    }

    fn top_n(self, sort_col: &str, n: u32) -> Self {
        self.sort([sort_col], SortMultipleOptions::default().with_order_descending(true))
            .limit(n)
    }

    fn running_total(self, value_col: &str, alias: &str) -> Self {
        self.with_column(col(value_col).cum_sum(false).alias(alias))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::Ingestor;

    fn sales_lf() -> LazyFrame {
        Ingestor::scan_csv("data/sales.csv").unwrap()
    }

    #[test]
    fn test_add_revenue() {
        let df = sales_lf().add_revenue().collect().unwrap();
        assert!(df.get_column_names().contains(&&PlSmallStr::from("revenue")));
        // First row: 2 * 1299.99 = 2599.98
        let rev = df.column("revenue").unwrap();
        let first = rev.f64().unwrap().get(0).unwrap();
        assert!((first - 2599.98).abs() < 0.01);
    }

    #[test]
    fn test_filter_completed() {
        let df = sales_lf().filter_completed().collect().unwrap();
        let statuses = df.column("status").unwrap();
        for s in statuses.str().unwrap().into_iter() {
            assert_eq!(s.unwrap(), "completed");
        }
    }

    #[test]
    fn test_revenue_by_category() {
        let df = sales_lf()
            .add_revenue()
            .filter_completed()
            .revenue_by("category")
            .collect()
            .unwrap();
        assert!(df.height() > 0);
        assert!(df.get_column_names().contains(&&PlSmallStr::from("total_revenue")));
    }

    #[test]
    fn test_revenue_by_region() {
        let df = sales_lf()
            .add_revenue()
            .filter_completed()
            .revenue_by("region")
            .collect()
            .unwrap();
        assert_eq!(df.height(), 4); // north, south, east, west
    }

    #[test]
    fn test_top_n() {
        let df = sales_lf()
            .add_revenue()
            .top_n("revenue", 5)
            .collect()
            .unwrap();
        assert_eq!(df.height(), 5);
    }

    #[test]
    fn test_filter_region() {
        let df = sales_lf()
            .filter_region("north")
            .collect()
            .unwrap();
        for r in df.column("region").unwrap().str().unwrap().into_iter() {
            assert_eq!(r.unwrap(), "north");
        }
    }
}
