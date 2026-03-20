//! Sales analytics report example.
//!
//! Run: cargo run -p analytics --example sales_report
//!
//! Demonstrates:
//! - Loading CSV data into the catalog
//! - Joining datasets (sales + customers)
//! - Revenue calculations, grouping, filtering
//! - Pipeline composition and lineage tracking

use analytics::{DataCatalog, Pipeline};
use polars::prelude::*;

fn data_path(file: &str) -> String {
    let manifest = env!("CARGO_MANIFEST_DIR");
    format!("{}/data/{}", manifest, file)
}

fn main() {
    let mut catalog = DataCatalog::new();

    // ── Ingest ────────────────────────────────────────────────
    println!("=== Data Ingestion ===\n");

    catalog.register_csv("sales", &data_path("sales.csv")).unwrap();
    catalog.register_csv("customers", &data_path("customers.csv")).unwrap();

    for name in catalog.list() {
        let meta = catalog.meta(name).unwrap();
        println!("  {} → {} rows × {} cols", name, meta.row_count, meta.column_count);
    }

    // ── Pipeline 1: Revenue by category ──────────────────────
    println!("\n=== Revenue by Category (completed orders) ===\n");

    let by_category = Pipeline::new("revenue_by_category")
        .source("sales", &catalog).unwrap()
        .filter_completed()
        .add_revenue_column()
        .revenue_by("category")
        .execute_into(&mut catalog, "revenue_by_category").unwrap();

    println!("{}\n", by_category);

    // ── Pipeline 2: Revenue by region ────────────────────────
    println!("=== Revenue by Region ===\n");

    let by_region = Pipeline::new("revenue_by_region")
        .source("sales", &catalog).unwrap()
        .filter_completed()
        .add_revenue_column()
        .revenue_by("region")
        .execute_into(&mut catalog, "revenue_by_region").unwrap();

    println!("{}\n", by_region);

    // ── Pipeline 3: Top 5 orders by revenue ──────────────────
    println!("=== Top 5 Orders ===\n");

    let top_orders = Pipeline::new("top_orders")
        .source("sales", &catalog).unwrap()
        .filter_completed()
        .add_revenue_column()
        .select(&["order_id", "product", "category", "quantity", "unit_price", "revenue", "region"])
        .sort(&["revenue"], true)
        .limit(5)
        .execute_into(&mut catalog, "top_5_orders").unwrap();

    println!("{}\n", top_orders);

    // ── Pipeline 4: Enriched sales (join with customers) ─────
    println!("=== Customer-Enriched Sales (Gold tier only) ===\n");

    let gold_sales = Pipeline::new("gold_customer_sales")
        .source("sales", &catalog).unwrap()
        .join("customers", &catalog, "customer_id", "customer_id").unwrap()
        .filter_completed()
        .filter(col("tier").eq(lit("gold")))
        .add_revenue_column()
        .select(&["order_id", "name", "product", "revenue", "tier", "order_date"])
        .sort(&["revenue"], true)
        .execute_into(&mut catalog, "gold_customer_sales").unwrap();

    println!("{}\n", gold_sales);

    // ── Pipeline 5: Revenue per customer tier ────────────────
    println!("=== Revenue by Customer Tier ===\n");

    let by_tier = Pipeline::new("revenue_by_tier")
        .source("sales", &catalog).unwrap()
        .join("customers", &catalog, "customer_id", "customer_id").unwrap()
        .filter_completed()
        .add_revenue_column()
        .revenue_by("tier")
        .execute_into(&mut catalog, "revenue_by_tier").unwrap();

    println!("{}\n", by_tier);

    // ── Catalog summary ──────────────────────────────────────
    println!("=== Data Catalog ===\n");

    for name in catalog.list() {
        let meta = catalog.meta(name).unwrap();
        println!("  {} ({} × {})", name, meta.row_count, meta.column_count);
        for step in &meta.lineage {
            println!("    └─ {}", step);
        }
    }
}
