//! CI / shell observability + developer insights (docs/ci-insights.md).
//!
//! Pulls what CI produced into the Postgres `insights` database:
//!
//! - [`github`]: GitHub Actions runs (+ attempt 1 of re-runs), jobs, steps,
//!   artifacts and commits over the REST API;
//! - [`ingest`]: `taskgraph-events-*` artifacts → the `TASKGRAPH` stream,
//!   `taskgraph-shell-scan*` artifacts → shell scan tables;
//! - [`classify`]: commit authors/assistants via `core_authorship`;
//! - [`warehouse`]: the `insights-warehouse` consumer folding `TASKGRAPH`
//!   into task runs / executions / commands;
//! - [`traces`]: finished runs → OTLP spans with their original timestamps;
//! - [`sync`]: one cycle over all of the above;
//! - [`store`]: every SQL statement (idempotent upserts).
//!
//! Grafana reads only the views/functions of `manifests/db/insights/schema.sql`.

pub mod classify;
pub mod error;
pub mod github;
pub mod ingest;
pub mod store;
pub mod sync;
pub mod traces;
pub mod warehouse;

pub use error::{InsightsError, InsightsResult};
pub use github::GitHub;
pub use store::Store;
pub use sync::{CycleReport, Stage, StageOutcome, StageReport, SyncConfig, Syncer};
pub use traces::TraceExporter;
