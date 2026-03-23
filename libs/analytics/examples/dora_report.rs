//! DORA metrics report example.
//!
//! Run: cargo run -p analytics --example dora_report
//!
//! Demonstrates:
//! - Loading deployment, incident, and PR data from CSV
//! - Computing all four DORA metrics with classifications
//! - Breaking down metrics by service, team, and severity
//! - Loading mock 3rd-party integration config

use analytics::dora;
use analytics::DataCatalog;

fn data_path(file: &str) -> String {
    let manifest = env!("CARGO_MANIFEST_DIR");
    format!("{}/data/{}", manifest, file)
}

fn main() {
    let mut catalog = DataCatalog::new();

    // ── Ingest DORA data sources ─────────────────────────────
    println!("=== Loading DORA Data Sources ===\n");

    catalog
        .register_csv("deployments", &data_path("deployments.csv"))
        .unwrap();
    catalog
        .register_csv("incidents", &data_path("incidents.csv"))
        .unwrap();
    catalog
        .register_csv("pull_requests", &data_path("pull_requests.csv"))
        .unwrap();

    for name in &["deployments", "incidents", "pull_requests"] {
        let meta = catalog.meta(name).unwrap();
        println!("  {} → {} rows × {} cols", name, meta.row_count, meta.column_count);
    }

    // ── Load mock integration config ─────────────────────────
    println!("\n=== Integration Sources (mock tokens) ===\n");

    let integrations_path = data_path("integrations.json");
    let integrations: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&integrations_path).unwrap()).unwrap();

    for (provider, _config) in integrations.as_object().unwrap() {
        if provider == "_comment" {
            continue;
        }
        println!("  ✓ {} configured", provider);
    }

    // ── DORA Summary ─────────────────────────────────────────
    println!("\n=== DORA Metrics Summary ===\n");

    let summary = dora::compute_dora_summary(&catalog).unwrap();
    println!("{}", summary);

    // ── Metric 1: Deployment Frequency ───────────────────────
    println!("=== Deployment Frequency ===\n");

    let df_freq = dora::deployment_frequency(&catalog).unwrap();
    println!("{}\n", df_freq);

    println!("By service:");
    let by_service = dora::deployment_frequency_by_service(&catalog).unwrap();
    println!("{}\n", by_service);

    // ── Metric 2: Lead Time for Changes ──────────────────────
    println!("=== Lead Time for Changes ===\n");

    let lt = dora::lead_time_for_changes(&catalog).unwrap();
    println!("{}\n", lt);

    println!("By team:");
    let by_team = dora::lead_time_by_team(&catalog).unwrap();
    println!("{}\n", by_team);

    // ── Metric 3: Change Failure Rate ────────────────────────
    println!("=== Change Failure Rate ===\n");

    let cfr = dora::change_failure_rate(&catalog).unwrap();
    println!("{}\n", cfr);

    println!("By service:");
    let cfr_svc = dora::change_failure_rate_by_service(&catalog).unwrap();
    println!("{}\n", cfr_svc);

    // ── Metric 4: Mean Time to Restore ───────────────────────
    println!("=== Mean Time to Restore (MTTR) ===\n");

    let mttr = dora::mean_time_to_restore(&catalog).unwrap();
    println!("{}\n", mttr);

    println!("By severity:");
    let by_sev = dora::mttr_by_severity(&catalog).unwrap();
    println!("{}\n", by_sev);

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
