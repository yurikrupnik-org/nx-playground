//! One sync cycle: runs/jobs/steps → artifacts → commits → warehouse drain →
//! derived flags → traces.
//!
//! Every stage records success or failure in `sync_state` and the cycle
//! moves on past a failing stage: a broken artifact download must not stop
//! commits from syncing. A GitHub rate limit is the exception — it blocks
//! every GitHub stage until the reset time (persisted, so a restart honours
//! it), while the stages that do not call GitHub still run.

use std::time::{Duration, Instant};

use async_nats::jetstream::Context;
use chrono::{DateTime, Utc};
use core_authorship::Registry;
use domain_taskgraph::NatsPublisher;
use tracing::{debug, info, warn};

use crate::classify::classify;
use crate::error::{InsightsError, InsightsResult};
use crate::github::{Commit, GitHub, GitHubError};
use crate::ingest::ArtifactIngest;
use crate::store::{Store, is_counted};
use crate::traces::{TraceExporter, export_pending};
use crate::warehouse;

/// Re-read this far behind a stage's cursor: re-runs of recent runs and
/// commits whose committer date predates their push.
const OVERLAP: chrono::Duration = chrono::Duration::days(3);
/// `sync_state` row carrying the GitHub rate-limit block.
const GITHUB: &str = "github";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Runs,
    Artifacts,
    Commits,
    Warehouse,
    Derived,
    Traces,
}

impl Stage {
    pub const ALL: [Stage; 6] = [
        Self::Runs,
        Self::Artifacts,
        Self::Commits,
        Self::Warehouse,
        Self::Derived,
        Self::Traces,
    ];

    /// `sync_state.source` / metric label.
    pub fn source(self) -> &'static str {
        match self {
            Self::Runs => "runs",
            Self::Artifacts => "artifacts",
            Self::Commits => "commits",
            Self::Warehouse => "warehouse",
            Self::Derived => "derived",
            Self::Traces => "traces",
        }
    }

    fn uses_github(self) -> bool {
        matches!(self, Self::Runs | Self::Artifacts | Self::Commits)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageOutcome {
    Ok {
        items: u64,
    },
    /// Nothing to do by configuration (e.g. OTLP export not configured).
    Skipped(String),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct StageReport {
    pub stage: Stage,
    pub outcome: StageOutcome,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct CycleReport {
    pub stages: Vec<StageReport>,
    pub elapsed: Duration,
}

impl CycleReport {
    /// No stage failed.
    pub fn is_ok(&self) -> bool {
        self.stages
            .iter()
            .all(|s| !matches!(s.outcome, StageOutcome::Failed(_)))
    }
}

#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// How far back the first cycle reads runs and commits.
    pub backfill_days: i64,
    /// Workflow paths whose runs count toward the scorecard.
    pub ci_workflows: Vec<String>,
}

enum Done {
    Items(u64, Option<DateTime<Utc>>),
    Skipped(String),
}

pub struct Syncer {
    store: Store,
    github: GitHub,
    jetstream: Option<Context>,
    publisher: Option<NatsPublisher>,
    exporter: Option<TraceExporter>,
    config: SyncConfig,
    registry: &'static Registry,
}

impl Syncer {
    /// Without NATS ([`Self::attach_nats`] not called or failed) the artifact
    /// and warehouse stages fail instead of losing events; `exporter` =
    /// `None` skips traces.
    pub fn new(
        store: Store,
        github: GitHub,
        exporter: Option<TraceExporter>,
        config: SyncConfig,
    ) -> Self {
        Self {
            store,
            github,
            jetstream: None,
            publisher: None,
            exporter,
            config,
            registry: core_authorship::registry(),
        }
    }

    /// Use `jetstream` for artifact events and the warehouse (creates the
    /// `TASKGRAPH` stream if absent).
    pub async fn attach_nats(&mut self, jetstream: Context) -> InsightsResult<()> {
        let publisher = NatsPublisher::new(jetstream.clone())
            .await
            .map_err(|e| InsightsError::Nats(e.to_string()))?;
        self.publisher = Some(publisher);
        self.jetstream = Some(jetstream);
        Ok(())
    }

    pub fn has_nats(&self) -> bool {
        self.jetstream.is_some()
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub async fn run_cycle(&self) -> CycleReport {
        let cycle_started = Instant::now();
        let mut report = CycleReport::default();
        let mut blocked_until = match self.store.sync_state(GITHUB).await {
            Ok(state) => state.blocked_until.filter(|t| *t > Utc::now()),
            Err(e) => {
                warn!(error = %e, "reading the GitHub rate-limit block");
                None
            }
        };
        for stage in Stage::ALL {
            let started = Instant::now();
            let result = match blocked_until {
                Some(until) if stage.uses_github() => {
                    Err(InsightsError::GitHub(GitHubError::RateLimited {
                        reset_at: until,
                    }))
                }
                _ => self.run_stage(stage).await,
            };
            let outcome = match result {
                Ok(Done::Items(items, cursor)) => match self
                    .store
                    .record_success(stage.source(), items, cursor)
                    .await
                {
                    Ok(()) => StageOutcome::Ok { items },
                    Err(e) => StageOutcome::Failed(format!("recording success: {e}")),
                },
                Ok(Done::Skipped(reason)) => {
                    debug!(stage = stage.source(), %reason, "stage skipped");
                    StageOutcome::Skipped(reason)
                }
                Err(e) => {
                    if let InsightsError::GitHub(GitHubError::RateLimited { reset_at }) = &e
                        && blocked_until.is_none()
                    {
                        blocked_until = Some(*reset_at);
                        if let Err(db) = self.store.block_github_until(*reset_at).await {
                            warn!(error = %db, "persisting the GitHub rate-limit block");
                        }
                    }
                    let message = e.to_string();
                    warn!(stage = stage.source(), error = %message, "sync stage failed");
                    if let Err(db) = self.store.record_failure(stage.source(), &message).await {
                        warn!(error = %db, "recording stage failure");
                    }
                    StageOutcome::Failed(message)
                }
            };
            report.stages.push(StageReport {
                stage,
                outcome,
                elapsed: started.elapsed(),
            });
        }
        report.elapsed = cycle_started.elapsed();
        info!(
            ok = report.is_ok(),
            elapsed_ms = report.elapsed.as_millis() as u64,
            "sync cycle finished"
        );
        report
    }

    async fn run_stage(&self, stage: Stage) -> InsightsResult<Done> {
        match stage {
            Stage::Runs => self.sync_runs().await,
            Stage::Artifacts => self.sync_artifacts().await,
            Stage::Commits => self.sync_commits().await,
            Stage::Warehouse => {
                let Some(jetstream) = &self.jetstream else {
                    return Err(InsightsError::Nats("NATS unavailable".into()));
                };
                let drained = warehouse::drain(jetstream, &self.store).await?;
                Ok(Done::Items(drained.applied, None))
            }
            Stage::Derived => Ok(Done::Items(self.store.refresh_derived_flags().await?, None)),
            Stage::Traces => match &self.exporter {
                Some(exporter) => Ok(Done::Items(
                    export_pending(&self.store, exporter).await?,
                    None,
                )),
                None => Ok(Done::Skipped("OTEL_EXPORTER_OTLP_ENDPOINT not set".into())),
            },
        }
    }

    /// Where a GitHub listing starts: the backfill horizon on the first
    /// cycle, a short overlap behind the cursor afterwards.
    fn since(&self, cursor: Option<DateTime<Utc>>, now: DateTime<Utc>) -> DateTime<Utc> {
        let horizon = now - chrono::Duration::days(self.config.backfill_days);
        cursor.map_or(horizon, |c| (c - OVERLAP).max(horizon))
    }

    async fn sync_runs(&self) -> InsightsResult<Done> {
        let now = Utc::now();
        let state = self.store.sync_state(Stage::Runs.source()).await?;
        let runs = self
            .github
            .workflow_runs(self.since(state.cursor_at, now), now)
            .await?;
        let mut items = 0;
        for run in &runs {
            let counted = is_counted(&run.path, &self.config.ci_workflows);
            self.store
                .upsert_run_attempt(run, run.attempt(), counted)
                .await?;
            items += 1;
            // first-pass rate is about attempt 1; the listing only shows the latest.
            if run.attempt() > 1 && !self.store.attempt_completed(run.id, 1).await? {
                let first = self.github.run_attempt(run.id, 1).await?;
                self.store.upsert_run_attempt(&first, 1, counted).await?;
                items += 1;
            }
        }
        self.store
            .refresh_ci_workflow_flags(&self.config.ci_workflows)
            .await?;
        for (run_id, attempt) in self.store.attempts_needing_jobs().await? {
            let jobs = match self.github.attempt_jobs(run_id, attempt).await {
                Ok(jobs) => jobs,
                Err(e) if e.is_gone() => Vec::new(),
                Err(e) => return Err(e.into()),
            };
            self.store.store_jobs(run_id, attempt, &jobs).await?;
        }
        Ok(Done::Items(items, Some(now)))
    }

    async fn sync_artifacts(&self) -> InsightsResult<Done> {
        let Some(publisher) = &self.publisher else {
            return Err(InsightsError::Nats(
                "NATS unavailable; artifacts wait until TASKGRAPH is reachable".into(),
            ));
        };
        let ingest = ArtifactIngest {
            github: &self.github,
            store: &self.store,
            publisher,
        };
        let mut items = 0;
        for (run_id, attempt) in self.store.runs_needing_artifacts().await? {
            items += ingest.ingest_run(run_id).await?;
            self.store.mark_artifacts_synced(run_id, attempt).await?;
        }
        Ok(Done::Items(items, None))
    }

    async fn sync_commits(&self) -> InsightsResult<Done> {
        let now = Utc::now();
        let state = self.store.sync_state(Stage::Commits.source()).await?;
        let branch = self.github.default_branch().await?;
        self.store
            .upsert_repository(self.github.repository(), &branch)
            .await?;

        let mut items = 0;
        for commit in self
            .github
            .commits(&branch, self.since(state.cursor_at, now))
            .await?
        {
            self.store
                .upsert_commit(&classify(self.registry, &commit), true)
                .await?;
            items += 1;
        }
        for (sha, pushed_to_default) in self.store.run_heads_without_commit().await? {
            match self.github.commit(&sha).await {
                Ok(commit) => {
                    self.store
                        .upsert_commit(&classify(self.registry, &commit), pushed_to_default)
                        .await?;
                    self.store_details(&commit).await?;
                    items += 1;
                }
                Err(e) if e.is_gone() => {
                    self.store
                        .record_sync_error(Stage::Commits.source(), Some(&sha), &e.to_string())
                        .await?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        for sha in self.store.commits_missing_details().await? {
            match self.github.commit(&sha).await {
                Ok(commit) => self.store_details(&commit).await?,
                Err(e) if e.is_gone() => self.store.mark_commit_details_unavailable(&sha).await?,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(Done::Items(items, Some(now)))
    }

    async fn store_details(&self, commit: &Commit) -> InsightsResult<()> {
        let (additions, deletions) = commit
            .stats
            .as_ref()
            .map_or((0, 0), |s| (s.additions, s.deletions));
        self.store
            .store_commit_details(
                &commit.sha,
                additions,
                deletions,
                commit.files.as_deref().unwrap_or_default(),
            )
            .await?;
        Ok(())
    }
}
