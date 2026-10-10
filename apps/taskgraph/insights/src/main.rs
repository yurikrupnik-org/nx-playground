//! `taskgraph_insights` — the CI / developer insights sync service.
//!
//! Every `INSIGHTS_SYNC_INTERVAL` it runs one `domain_insights` sync cycle
//! (GitHub Actions runs/jobs/steps → CI artifacts → commits → TASKGRAPH
//! warehouse → derived flags → OTLP traces) into Postgres `insights`, and
//! serves:
//!
//! - `GET /healthz` — liveness
//! - `GET /readyz` — at least one cycle completed with no failed stage
//! - `GET /metrics` — `insights_*` series + the axum_helpers HTTP series
//!
//! `--once` runs a single cycle, prints a per-stage report and exits
//! non-zero when a stage failed (the host-side `task insights-sync`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::get;
use axum_helpers::{create_app, init_metrics, metrics_router, track_metrics};
use clap::Parser;
use config::AppConfig;
use core_config::{FromEnv, app_info};
use domain_insights::{
    CycleReport, GitHub, StageOutcome, Store, SyncConfig, Syncer, TraceExporter,
};
use eyre::{Result, WrapErr, bail};
use tokio::sync::watch;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

mod config;

#[derive(Parser)]
#[command(
    version,
    about = "Sync GitHub Actions, commits and CI artifacts into Postgres `insights`"
)]
struct Cli {
    /// Run one sync cycle, print the report and exit (non-zero if a stage failed).
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = AppConfig::from_env()?;
    let _tracing_guard = core_config::tracing::init_tracing(&config.environment, app_info!());

    let store = Store::connect(&config.database_url)
        .await
        .wrap_err("connecting to Postgres (DATABASE_URL)")?;
    let github = GitHub::new(&config.github_token, &config.github_repository)?;
    let exporter =
        config
            .otlp_endpoint
            .as_deref()
            .and_then(|endpoint| match TraceExporter::new(endpoint) {
                Ok(exporter) => Some(exporter),
                Err(e) => {
                    warn!(error = %e, "CI trace export disabled");
                    None
                }
            });
    let mut syncer = Syncer::new(
        store,
        github,
        exporter,
        SyncConfig {
            backfill_days: config.backfill_days,
            ci_workflows: config.ci_workflows.clone(),
        },
    );
    attach_nats(&mut syncer, &config.nats_url).await;

    if cli.once {
        let report = syncer.run_cycle().await;
        print_report(&report);
        let failed = report
            .stages
            .iter()
            .filter(|s| matches!(s.outcome, StageOutcome::Failed(_)))
            .count();
        if failed > 0 {
            bail!("{failed} sync stage(s) failed");
        }
        return Ok(());
    }

    let metrics = init_metrics().wrap_err("installing Prometheus recorder")?;
    describe_metrics();
    let ready = Arc::new(AtomicBool::new(false));
    let (stop, stopped) = watch::channel(false);
    let sync = tokio::spawn(sync_loop(
        syncer,
        config.nats_url.clone(),
        config.sync_interval,
        ready.clone(),
        stopped,
    ));

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .with_state(ready)
        .layer(middleware::from_fn(track_metrics))
        .layer(TraceLayer::new_for_http())
        .merge(metrics_router(metrics));

    info!(addr = %config.server.addr(), interval_s = config.sync_interval.as_secs(), "taskgraph-insights listening");
    create_app(app, &config.server)
        .await
        .wrap_err("server error")?;

    // SIGTERM/SIGINT ended the server: stop the loop. A cycle in flight is
    // dropped at its next await; every write is idempotent, so the next start
    // resumes cleanly.
    let _ = stop.send(true);
    if tokio::time::timeout(Duration::from_secs(10), sync)
        .await
        .is_err()
    {
        warn!("sync loop did not stop within 10s");
    }
    Ok(())
}

/// Bounded connect: a missing NATS fails the artifact/warehouse stages (and
/// is retried every cycle by the loop) rather than blocking startup.
async fn attach_nats(syncer: &mut Syncer, nats_url: &str) {
    match messaging::nats::jetstream(nats_url).await {
        Ok(jetstream) => match syncer.attach_nats(jetstream).await {
            Ok(()) => info!(%nats_url, "connected to NATS"),
            Err(e) => warn!(%nats_url, error = %e, "TASKGRAPH unavailable"),
        },
        Err(e) => warn!(%nats_url, error = %e, "NATS unreachable"),
    }
}

async fn sync_loop(
    mut syncer: Syncer,
    nats_url: String,
    interval: Duration,
    ready: Arc<AtomicBool>,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        if !syncer.has_nats() {
            attach_nats(&mut syncer, &nats_url).await;
        }
        tokio::select! {
            report = syncer.run_cycle() => {
                record_metrics(&report);
                if report.is_ok() {
                    ready.store(true, Ordering::Relaxed);
                }
            }
            _ = stop.changed() => return,
        }
        tokio::select! {
            () = tokio::time::sleep(interval) => {}
            _ = stop.changed() => return,
        }
    }
}

async fn readyz(State(ready): State<Arc<AtomicBool>>) -> (StatusCode, &'static str) {
    if ready.load(Ordering::Relaxed) {
        (StatusCode::OK, "ready")
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "no successful sync cycle yet",
        )
    }
}

fn describe_metrics() {
    metrics::describe_counter!(
        "insights_sync_cycles_total",
        "Sync cycles by outcome (ok | failed)"
    );
    metrics::describe_histogram!(
        "insights_sync_duration_seconds",
        metrics::Unit::Seconds,
        "Wall time of one sync cycle"
    );
    metrics::describe_counter!(
        "insights_items_total",
        "Items a sync stage processed, by source"
    );
    metrics::describe_gauge!(
        "insights_last_success_timestamp_seconds",
        metrics::Unit::Seconds,
        "Unix time of the last successful run of a sync stage, by source"
    );
}

fn record_metrics(report: &CycleReport) {
    let outcome = if report.is_ok() { "ok" } else { "failed" };
    metrics::counter!("insights_sync_cycles_total", "outcome" => outcome).increment(1);
    metrics::histogram!("insights_sync_duration_seconds").record(report.elapsed.as_secs_f64());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    for stage in &report.stages {
        if let StageOutcome::Ok { items } = stage.outcome {
            let source = stage.stage.source();
            metrics::counter!("insights_items_total", "source" => source).increment(items);
            metrics::gauge!("insights_last_success_timestamp_seconds", "source" => source).set(now);
        }
    }
}

fn print_report(report: &CycleReport) {
    for stage in &report.stages {
        let outcome = match &stage.outcome {
            StageOutcome::Ok { items } => format!("ok      {items} items"),
            StageOutcome::Skipped(reason) => format!("skipped {reason}"),
            StageOutcome::Failed(error) => format!("FAILED  {error}"),
        };
        println!(
            "{:<10} {:>7.1}s  {outcome}",
            stage.stage.source(),
            stage.elapsed.as_secs_f64()
        );
    }
    println!(
        "cycle      {:>7.1}s  {}",
        report.elapsed.as_secs_f64(),
        if report.is_ok() { "ok" } else { "failed" }
    );
}
