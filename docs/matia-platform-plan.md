# Zerg Analytics — Mini Matia Platform Plan

A self-hosted, Rust-native data platform inspired by [Matia.io](https://www.matia.io/).
Built on Polars for in-process analytics, with the existing Zerg infrastructure
(NATS, Dapr, KEDA, K8s) for orchestration.

## What Matia Does

Matia is a unified DataOps platform with 6 pillars:

1. **Data Ingestion (ETL)** — ingest from 100+ sources into warehouses
2. **Reverse ETL** — push processed data back to SaaS tools
3. **Data Catalog** — discover, organize, govern datasets
4. **Data Lineage** — column-level tracing from source to AI models
5. **Observability** — monitor data quality, freshness, pipeline health
6. **Connectors** — pluggable source/destination adapters

## What We Build (Scope)

A **self-hosted subset** tailored to our monorepo:

| Matia Feature | Our Implementation | Priority |
|---|---|---|
| Ingestion | Polars readers (CSV, JSON, Parquet) + DB connectors | P0 - Done |
| Catalog | In-memory + persisted dataset registry with schema/lineage | P0 - Done |
| Transformations | Composable Pipeline API with Polars LazyFrame | P0 - Done |
| Lineage | Track source → transform → output chains | P0 - Done |
| Observability | Data quality checks, row counts, schema drift detection | P1 |
| Reverse ETL | Push results to PG/Mongo/NATS via existing db-worker | P1 |
| Connectors (Source) | PostgreSQL, MongoDB, NATS, Qdrant, InfluxDB, S3 | P1 |
| Connectors (Sink) | BigQuery, Bigtable, PostgreSQL, MongoDB, S3, NATS | P1 |
| Scheduling | Cron-based pipeline execution via NATS + KEDA | P2 |
| API | REST endpoints for catalog, pipeline runs, results | P2 |
| UI | Web dashboard for catalog browsing, pipeline DAGs | P3 |

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        Zerg Analytics                           │
│                                                                 │
│  ┌──────────┐  ┌──────────────┐  ┌────────────┐  ┌──────────┐  │
│  │ Ingestor │  │  Pipeline    │  │  Catalog   │  │ Exporter │  │
│  │          │  │              │  │            │  │          │  │
│  │ CSV      │  │ LazyFrame    │  │ Metadata   │  │ CSV      │  │
│  │ JSON     │─▶│ transforms   │─▶│ Schema     │─▶│ Parquet  │  │
│  │ Parquet  │  │ joins        │  │ Lineage    │  │ PG/Mongo │  │
│  │ PG query │  │ aggregations │  │ Quality    │  │ NATS/S3  │  │
│  │ Mongo    │  │ filters      │  │ Freshness  │  │ BigQuery │  │
│  │ NATS     │  │              │  │            │  │ Bigtable │  │
│  └──────────┘  └──────────────┘  └────────────┘  └──────────┘  │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Scheduler                             │   │
│  │  NATS event → Pipeline run → Catalog update → Export     │   │
│  │  KEDA scales pipeline workers based on queue depth       │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    API + UI                              │   │
│  │  GET  /analytics/catalog       → list datasets           │   │
│  │  GET  /analytics/catalog/:name → dataset details+schema  │   │
│  │  POST /analytics/pipelines     → run a pipeline          │   │
│  │  GET  /analytics/lineage/:name → full lineage graph      │   │
│  │  GET  /analytics/quality/:name → quality report          │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Implementation Phases

### Phase 0: Analytics Core (Done)

**Crate**: `libs/analytics/`

What exists today:
- `Ingestor` — read/write CSV, JSON, Parquet
- `Transforms` trait — add_revenue, filter, group_by, top_n, running_total
- `Pipeline` — composable builder: source → filter → join → aggregate → execute
- `DataCatalog` — register datasets, track schema + lineage, describe
- Sample data — `data/sales.csv`, `data/customers.csv`
- 21 passing tests + runnable example (`cargo run -p analytics --example sales_report`)

### Phase 1: DB Connectors + Data Quality

**Files**:
```
libs/analytics/src/
  connectors/
    mod.rs
    postgres.rs     # SELECT → DataFrame via sqlx
    mongo.rs        # collection.find() → DataFrame
    nats.rs         # consume stream → DataFrame (batch window)
  sinks/
    mod.rs
    postgres.rs     # DataFrame → INSERT INTO / COPY via sqlx
    mongo.rs        # DataFrame → collection.insert_many()
    bigquery.rs     # DataFrame → BigQuery table via gcp-bigquery-client
    bigtable.rs     # DataFrame → Bigtable rows via google-cloud-bigtable
    nats.rs         # DataFrame rows → NATS topic (JSON per row)
    s3.rs           # DataFrame → Parquet/CSV on S3 (via aws-sdk-s3)
  quality/
    mod.rs
    checks.rs       # null counts, uniqueness, range, regex patterns
    freshness.rs    # staleness detection (last updated vs threshold)
    schema.rs       # schema drift detection between runs
    report.rs       # QualityReport struct
```

**Key types**:
```rust
/// Register a PG query as a dataset
catalog.register_query("active_users",
    "SELECT * FROM users WHERE status = 'active'",
    &pg_pool,
).await?;

/// Register a Mongo collection
catalog.register_collection("tasks",
    &mongo_db, "tasks",
    doc! { "status": "done" },
).await?;

/// Run quality checks
let report = QualityCheck::new(&catalog, "sales")
    .not_null(&["order_id", "customer_id", "unit_price"])
    .unique("order_id")
    .range("unit_price", 0.0..10000.0)
    .freshness("order_date", Duration::days(7))
    .run()?;

assert!(report.passed());
```

### Phase 2: Reverse ETL + Scheduled Pipelines

**Reverse ETL** — push pipeline results back to operational systems:

```rust
// Pipeline output → PostgreSQL table
Pipeline::new("daily_summary")
    .source("sales", &catalog).unwrap()
    .filter_completed()
    .add_revenue_column()
    .revenue_by("category")
    .export_to_postgres(&pg_pool, "analytics.daily_summary")
    .await?;

// Pipeline output → NATS topic (for downstream consumers)
Pipeline::new("anomaly_detection")
    .source("metrics", &catalog).unwrap()
    .filter(col("value").gt(col("threshold")))
    .export_to_nats(&jetstream, "alerts.anomalies")
    .await?;

// Pipeline output → Dapr pub/sub → db-worker → any backend
Pipeline::new("sync_to_mongo")
    .source("enriched_tasks", &catalog).unwrap()
    .export_via_dapr(&pubsub, "db.tasks.mongo")
    .await?;

// Pipeline output → BigQuery table
Pipeline::new("daily_analytics")
    .source("sales", &catalog).unwrap()
    .filter_completed()
    .add_revenue_column()
    .revenue_by("category")
    .export_to_bigquery(&bq_client, "project.dataset.daily_revenue")
    .await?;

// Pipeline output → Bigtable (wide-column for time-series / lookups)
Pipeline::new("user_features")
    .source("active_users", &catalog).unwrap()
    .export_to_bigtable(&bt_client, "user-features", "cf1")
    .await?;

// Pipeline output → S3 as Parquet (data lake)
Pipeline::new("archive_metrics")
    .source("metrics_1h", &catalog).unwrap()
    .export_to_s3(&s3_client, "s3://data-lake/metrics/", ExportFormat::Parquet)
    .await?;
```

**Scheduling** via NATS + existing worker pattern:

```rust
/// A scheduled pipeline job
#[derive(Serialize, Deserialize)]
struct PipelineJob {
    pipeline_name: String,
    schedule: String,        // cron expression
    sources: Vec<String>,    // catalog dataset names
    output: OutputTarget,    // where to write results
}

enum OutputTarget {
    Catalog(String),                                    // register in catalog
    Postgres { table: String },
    Mongo { database: String, collection: String },
    Nats { topic: String },
    Parquet { path: String },
    BigQuery { table_id: String },                      // project.dataset.table
    Bigtable { table: String, column_family: String },
    S3 { bucket: String, prefix: String, format: ExportFormat },
}

#[derive(Clone, Serialize, Deserialize)]
enum ExportFormat { Parquet, Csv, Json }
```

**New app**: `apps/zerg/analytics-worker/`
- Follows email-nats pattern (`NatsWorker<PipelineJob, PipelineProcessor>`)
- KEDA scales based on pipeline job queue depth
- Runs pipelines, writes results, updates catalog

### Phase 3: Analytics API

**New routes in `apps/zerg/api/`**:

```
GET    /api/analytics/catalog              → list all datasets
GET    /api/analytics/catalog/:name        → dataset metadata + schema
GET    /api/analytics/catalog/:name/sample → first N rows as JSON
GET    /api/analytics/catalog/:name/stats  → summary statistics
DELETE /api/analytics/catalog/:name        → remove dataset

POST   /api/analytics/pipelines            → run a pipeline (async)
GET    /api/analytics/pipelines/:id/status → check run status
GET    /api/analytics/pipelines/:id/result → get results

GET    /api/analytics/lineage/:name        → lineage graph (JSON)
GET    /api/analytics/quality/:name        → latest quality report

POST   /api/analytics/query               → ad-hoc Polars SQL query
```

**AppState addition**:
```rust
pub struct AppState {
    // ... existing fields ...
    pub analytics_catalog: Arc<RwLock<DataCatalog>>,
}
```

### Phase 4: Web UI

Lightweight dashboard (in `apps/zerg/web/`):

- **Catalog browser** — searchable table of datasets with schema, row counts, freshness
- **Pipeline DAG** — visual graph of source → transforms → output
- **Lineage explorer** — click a dataset, see full upstream/downstream chain
- **Quality dashboard** — green/yellow/red checks per dataset
- **Query editor** — write Polars SQL, see results as a table
- **Schedule manager** — CRUD for scheduled pipeline runs

Tech: React (already in `apps/zerg/web/`) + the analytics API endpoints.

## Crate Dependency Graph

```
libs/analytics/              ← Polars core (no DB deps)
  ↑
libs/analytics-connectors/   ← Source connectors (PG, Mongo, NATS → DataFrame)
  ↑                            depends on: analytics, database
libs/analytics-sinks/        ← Output connectors (DataFrame → PG, Mongo, BigQuery, Bigtable, S3, NATS)
  ↑                            depends on: analytics
  ↑                            optional deps: gcp-bigquery-client, google-cloud-bigtable, aws-sdk-s3
apps/matia/api/              ← REST API for catalog/pipelines
  ↑                            depends on: analytics, analytics-connectors, analytics-sinks
apps/matia/worker/           ← Scheduled pipeline execution
  ↑                            depends on: analytics, analytics-connectors, analytics-sinks, messaging
apps/matia/web/              ← UI dashboard
```

## Key Design Decisions

1. **Polars over DataFusion/DuckDB** — Polars has the best Rust-native DataFrame API,
   lazy execution, and doesn't require a query engine process. DataFusion is better
   for SQL-first workloads; can add later via `polars-sql` feature.

2. **Catalog persistence** — Phase 0 is in-memory. Phase 1 adds persistence via
   PostgreSQL (catalog metadata table) or a JSON file. The catalog is lightweight
   (metadata only, not data copies).

3. **Separation of analytics core from connectors** — The `analytics` crate has zero
   DB dependencies. DB connectors live in a separate crate so the core stays lean
   and testable without Docker.

4. **Reuse existing infrastructure** — Reverse ETL uses db-worker via Dapr pub/sub.
   Scheduling uses NatsWorker. Scaling uses KEDA. No new infrastructure.

5. **Polars SQL for ad-hoc queries** — The `sql` feature enables `polars-sql` for
   users who prefer SQL over the DataFrame API. Queries run against catalog datasets.

## Running Today

```bash
# Run all tests (21 tests)
cargo test -p analytics

# Run the sales report example
cargo run -p analytics --example sales_report

# Output:
#   Revenue by category, region, customer tier
#   Top 5 orders
#   Gold-tier customer analysis
#   Full catalog with lineage
```
