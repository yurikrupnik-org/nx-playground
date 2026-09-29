//! Every SQL statement of the insights service (Postgres `insights`,
//! `manifests/db/insights`).
//!
//! Writers are idempotent upserts keyed by natural ids — GitHub run/job ids,
//! commit shas, taskgraph `event_id`/`run_id`/`instance`, `scan_id` — so a
//! sync that is re-run, interrupted or fed the same data twice converges on
//! the same rows.

use chrono::{DateTime, Utc};
use contract_taskgraph::shell::ShellScan;
use contract_taskgraph::{EventBody, InvokerKind, TaskgraphEvent};
use serde::Serialize;
use sqlx::postgres::PgPoolOptions;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::classify::ClassifiedCommit;
use crate::github::{CommitFile, Job, WorkflowRun};

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

/// Resume point and GitHub block of one sync stage.
#[derive(Debug, Clone, Default, FromRow)]
pub struct SyncState {
    pub cursor_at: Option<DateTime<Utc>>,
    pub blocked_until: Option<DateTime<Utc>>,
}

/// One consumed artifact.
#[derive(Debug, Clone)]
pub struct ArtifactLedger<'a> {
    pub artifact_id: i64,
    pub run_id: i64,
    pub name: &'a str,
    /// `events` | `shell_scan`.
    pub kind: &'a str,
    pub items: i32,
    pub malformed: i32,
    /// Set when the artifact is permanently unusable (expired, corrupt).
    pub error: Option<&'a str>,
}

/// What [`Store::apply_events`] did with a batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Applied {
    pub applied: u64,
    /// Already applied earlier (redelivery or re-published artifact).
    pub duplicates: u64,
}

/// A completed, counted run attempt waiting for its trace.
#[derive(Debug, Clone, FromRow)]
pub struct TraceAttempt {
    pub run_id: i64,
    pub attempt: i32,
    pub workflow: String,
    pub workflow_path: String,
    pub event: String,
    pub branch: Option<String>,
    pub head_sha: String,
    pub conclusion: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub actor: Option<String>,
    pub html_url: String,
    pub author: Option<String>,
    pub author_kind: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct TraceJob {
    pub job_id: i64,
    pub name: String,
    pub conclusion: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub runner: Option<String>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct TraceStep {
    pub job_id: i64,
    pub number: i32,
    pub name: String,
    pub conclusion: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct TraceTaskRun {
    pub run_id: Uuid,
    pub target: Option<String>,
    pub ci_job: Option<String>,
    pub outcome: Option<String>,
    pub exit_code: Option<i32>,
    pub invoker_kind: Option<String>,
    pub agent: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct TraceExecution {
    pub run_id: Uuid,
    pub instance: i32,
    pub task: String,
    pub parent: Option<i32>,
    pub via: Option<String>,
    pub outcome: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
}

/// `workflow_path` in the configured list; a `@ref` suffix is ignored.
pub fn is_counted(workflow_path: &str, ci_workflows: &[String]) -> bool {
    let path = workflow_path.split('@').next().unwrap_or(workflow_path);
    ci_workflows.iter().any(|w| w == path)
}

/// The snake_case wire name serde gives a contract enum (`up_to_date`).
fn wire_name<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn to_i32(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

impl Store {
    pub async fn connect(url: &str) -> sqlx::Result<Self> {
        let pool = PgPoolOptions::new().max_connections(5).connect(url).await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // --- sync ledger ---------------------------------------------------------

    pub async fn sync_state(&self, source: &str) -> sqlx::Result<SyncState> {
        let row: Option<SyncState> =
            sqlx::query_as("SELECT cursor_at, blocked_until FROM sync_state WHERE source = $1")
                .bind(source)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.unwrap_or_default())
    }

    /// A stage finished. `cursor_at` = `None` keeps the stored cursor.
    pub async fn record_success(
        &self,
        source: &str,
        items: u64,
        cursor_at: Option<DateTime<Utc>>,
    ) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO sync_state (source, last_success_at, last_items, cursor_at)
             VALUES ($1, now(), $2, $3)
             ON CONFLICT (source) DO UPDATE SET
                 last_success_at = now(),
                 last_items = EXCLUDED.last_items,
                 cursor_at = COALESCE(EXCLUDED.cursor_at, sync_state.cursor_at)",
        )
        .bind(source)
        .bind(to_i64(items))
        .bind(cursor_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_failure(&self, source: &str, error: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO sync_state (source, last_error, last_error_at) VALUES ($1, $2, now())
             ON CONFLICT (source) DO UPDATE SET
                 last_error = EXCLUDED.last_error,
                 last_error_at = EXCLUDED.last_error_at",
        )
        .bind(source)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stop calling GitHub until `until` (rate limit).
    pub async fn block_github_until(&self, until: DateTime<Utc>) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO sync_state (source, blocked_until, last_error, last_error_at)
             VALUES ('github', $1, $2, now())
             ON CONFLICT (source) DO UPDATE SET
                 blocked_until = EXCLUDED.blocked_until,
                 last_error = EXCLUDED.last_error,
                 last_error_at = EXCLUDED.last_error_at",
        )
        .bind(until)
        .bind(format!("rate limited until {until}"))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_sync_error(
        &self,
        source: &str,
        subject: Option<&str>,
        error: &str,
    ) -> sqlx::Result<()> {
        sqlx::query("INSERT INTO sync_errors (source, subject, error) VALUES ($1, $2, $3)")
            .bind(source)
            .bind(subject)
            .bind(error)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_repository(
        &self,
        full_name: &str,
        default_branch: &str,
    ) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO repository (full_name, default_branch) VALUES ($1, $2)
             ON CONFLICT (full_name) DO UPDATE SET
                 default_branch = EXCLUDED.default_branch,
                 updated_at = now()",
        )
        .bind(full_name)
        .bind(default_branch)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // --- GitHub Actions ------------------------------------------------------

    /// Store `run` as attempt `attempt`. A completed attempt keeps the
    /// completion time it already has (the latest job completion, once jobs
    /// are synced).
    pub async fn upsert_run_attempt(
        &self,
        run: &WorkflowRun,
        attempt: i32,
        ci_workflow: bool,
    ) -> sqlx::Result<()> {
        let status = run.status.as_deref().unwrap_or("unknown");
        let completed_at = run.is_completed().then_some(run.updated_at);
        sqlx::query(
            "INSERT INTO ci_run_attempts (
                 run_id, attempt, workflow_id, workflow, workflow_path, ci_workflow, run_number,
                 event, branch, head_sha, status, conclusion, created_at, started_at, updated_at,
                 completed_at, actor, triggering_actor, html_url)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
             ON CONFLICT (run_id, attempt) DO UPDATE SET
                 workflow_id = EXCLUDED.workflow_id,
                 workflow = EXCLUDED.workflow,
                 workflow_path = EXCLUDED.workflow_path,
                 ci_workflow = EXCLUDED.ci_workflow,
                 run_number = EXCLUDED.run_number,
                 event = EXCLUDED.event,
                 branch = EXCLUDED.branch,
                 head_sha = EXCLUDED.head_sha,
                 status = EXCLUDED.status,
                 conclusion = EXCLUDED.conclusion,
                 created_at = EXCLUDED.created_at,
                 started_at = EXCLUDED.started_at,
                 updated_at = EXCLUDED.updated_at,
                 completed_at = COALESCE(ci_run_attempts.completed_at, EXCLUDED.completed_at),
                 actor = EXCLUDED.actor,
                 triggering_actor = EXCLUDED.triggering_actor,
                 html_url = EXCLUDED.html_url,
                 synced_at = now()",
        )
        .bind(run.id)
        .bind(attempt)
        .bind(run.workflow_id)
        .bind(run.name.as_deref().unwrap_or(&run.path))
        .bind(&run.path)
        .bind(ci_workflow)
        .bind(run.run_number)
        .bind(&run.event)
        .bind(run.head_branch.as_deref())
        .bind(&run.head_sha)
        .bind(status)
        .bind(run.conclusion.as_deref())
        .bind(run.created_at)
        .bind(run.run_started_at)
        .bind(run.updated_at)
        .bind(completed_at)
        .bind(run.actor.as_ref().and_then(|a| a.login.as_deref()))
        .bind(run.triggering_actor.as_ref().and_then(|a| a.login.as_deref()))
        .bind(&run.html_url)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Re-evaluate `ci_workflow` against the current configuration.
    pub async fn refresh_ci_workflow_flags(&self, ci_workflows: &[String]) -> sqlx::Result<u64> {
        let done = sqlx::query(
            "UPDATE ci_run_attempts SET ci_workflow = (split_part(workflow_path, '@', 1) = ANY($1))
             WHERE ci_workflow IS DISTINCT FROM (split_part(workflow_path, '@', 1) = ANY($1))",
        )
        .bind(ci_workflows)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }

    pub async fn attempt_completed(&self, run_id: i64, attempt: i32) -> sqlx::Result<bool> {
        let row: Option<(bool,)> = sqlx::query_as(
            "SELECT status = 'completed' FROM ci_run_attempts WHERE run_id = $1 AND attempt = $2",
        )
        .bind(run_id)
        .bind(attempt)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some_and(|r| r.0))
    }

    pub async fn attempts_needing_jobs(&self) -> sqlx::Result<Vec<(i64, i32)>> {
        sqlx::query_as(
            "SELECT run_id, attempt FROM ci_run_attempts
             WHERE status = 'completed' AND NOT jobs_synced
             ORDER BY created_at, run_id, attempt",
        )
        .fetch_all(&self.pool)
        .await
    }

    /// Jobs + steps of one completed attempt; marks the attempt's jobs synced
    /// and its completion as the latest job completion.
    pub async fn store_jobs(&self, run_id: i64, attempt: i32, jobs: &[Job]) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        for job in jobs {
            sqlx::query(
                "INSERT INTO ci_jobs (job_id, run_id, attempt, name, status, conclusion, created_at,
                     started_at, completed_at, runner, labels, html_url)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 ON CONFLICT (job_id) DO UPDATE SET
                     name = EXCLUDED.name,
                     status = EXCLUDED.status,
                     conclusion = EXCLUDED.conclusion,
                     created_at = EXCLUDED.created_at,
                     started_at = EXCLUDED.started_at,
                     completed_at = EXCLUDED.completed_at,
                     runner = EXCLUDED.runner,
                     labels = EXCLUDED.labels,
                     html_url = EXCLUDED.html_url",
            )
            .bind(job.id)
            .bind(run_id)
            .bind(attempt)
            .bind(&job.name)
            .bind(&job.status)
            .bind(job.conclusion.as_deref())
            .bind(job.created_at)
            .bind(job.started_at)
            .bind(job.completed_at)
            .bind(job.runner_name.as_deref())
            .bind(&job.labels)
            .bind(job.html_url.as_deref())
            .execute(&mut *tx)
            .await?;

            if job.steps.is_empty() {
                continue;
            }
            let numbers: Vec<i32> = job.steps.iter().map(|s| s.number).collect();
            let names: Vec<&str> = job.steps.iter().map(|s| s.name.as_str()).collect();
            let statuses: Vec<&str> = job.steps.iter().map(|s| s.status.as_str()).collect();
            let conclusions: Vec<Option<&str>> =
                job.steps.iter().map(|s| s.conclusion.as_deref()).collect();
            let started: Vec<Option<DateTime<Utc>>> =
                job.steps.iter().map(|s| s.started_at).collect();
            let completed: Vec<Option<DateTime<Utc>>> =
                job.steps.iter().map(|s| s.completed_at).collect();
            sqlx::query(
                "INSERT INTO ci_steps (job_id, number, name, status, conclusion, started_at, completed_at)
                 SELECT $1, * FROM UNNEST($2::int[], $3::text[], $4::text[], $5::text[],
                                          $6::timestamptz[], $7::timestamptz[])
                 ON CONFLICT (job_id, number) DO UPDATE SET
                     name = EXCLUDED.name,
                     status = EXCLUDED.status,
                     conclusion = EXCLUDED.conclusion,
                     started_at = EXCLUDED.started_at,
                     completed_at = EXCLUDED.completed_at",
            )
            .bind(job.id)
            .bind(&numbers)
            .bind(&names)
            .bind(&statuses)
            .bind(&conclusions)
            .bind(&started)
            .bind(&completed)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE ci_run_attempts SET
                 jobs_synced = true,
                 completed_at = COALESCE(
                     (SELECT max(j.completed_at) FROM ci_jobs j WHERE j.run_id = $1 AND j.attempt = $2),
                     completed_at)
             WHERE run_id = $1 AND attempt = $2",
        )
        .bind(run_id)
        .bind(attempt)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }

    // --- artifacts -----------------------------------------------------------

    /// Latest attempts that completed but whose artifacts were not consumed.
    pub async fn runs_needing_artifacts(&self) -> sqlx::Result<Vec<(i64, i32)>> {
        sqlx::query_as(
            "SELECT r.run_id, r.attempt FROM ci_run_attempts r
             WHERE r.status = 'completed' AND NOT r.artifacts_synced
               AND r.attempt = (SELECT max(x.attempt) FROM ci_run_attempts x WHERE x.run_id = r.run_id)
             ORDER BY r.created_at, r.run_id",
        )
        .fetch_all(&self.pool)
        .await
    }

    /// The artifact listing covers every attempt of the run.
    pub async fn mark_artifacts_synced(&self, run_id: i64, attempt: i32) -> sqlx::Result<()> {
        sqlx::query(
            "UPDATE ci_run_attempts SET artifacts_synced = true WHERE run_id = $1 AND attempt <= $2",
        )
        .bind(run_id)
        .bind(attempt)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn is_artifact_ingested(&self, artifact_id: i64) -> sqlx::Result<bool> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT artifact_id FROM ingested_artifacts WHERE artifact_id = $1")
                .bind(artifact_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    pub async fn ledger_artifact(&self, entry: &ArtifactLedger<'_>) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO ingested_artifacts (artifact_id, run_id, name, kind, items, malformed, error)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (artifact_id) DO NOTHING",
        )
        .bind(entry.artifact_id)
        .bind(entry.run_id)
        .bind(entry.name)
        .bind(entry.kind)
        .bind(entry.items)
        .bind(entry.malformed)
        .bind(entry.error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Store a scan once; `false` when `scan_id` was already stored (scans
    /// are immutable).
    pub async fn store_shell_scan(
        &self,
        scan: &ShellScan,
        artifact_id: Option<i64>,
    ) -> sqlx::Result<bool> {
        let mut tx = self.pool.begin().await?;
        let ci = scan.ci.as_ref();
        let inserted = sqlx::query(
            "INSERT INTO shell_scans (scan_id, scanned_at, sha, tool, provider, ci_run_id, ci_attempt,
                 ci_job, artifact_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (scan_id) DO NOTHING",
        )
        .bind(scan.scan_id)
        .bind(scan.scanned_at)
        .bind(scan.sha.as_deref())
        .bind(&scan.tool)
        .bind(ci.map(|c| c.provider.as_str()))
        .bind(ci.map(|c| c.run_id.as_str()))
        .bind(ci.and_then(|c| c.run_attempt).map(to_i32))
        .bind(ci.and_then(|c| c.job.as_deref()))
        .bind(artifact_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if !inserted {
            tx.rollback().await?;
            return Ok(false);
        }

        let s = &scan.sources;
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        let kinds: Vec<Option<String>> = s.iter().map(|x| wire_name(&x.kind)).collect();
        let paths: Vec<&str> = s.iter().map(|x| x.path.as_str()).collect();
        let tasks: Vec<Option<&str>> = s.iter().map(|x| x.task.as_deref()).collect();
        let idx: Vec<Option<i32>> = s.iter().map(|x| x.index.map(to_i32)).collect();
        let lines: Vec<i32> = s.iter().map(|x| to_i32(x.lines)).collect();
        let branches: Vec<i32> = s.iter().map(|x| to_i32(x.branches)).collect();
        let digests: Vec<&str> = s.iter().map(|x| x.digest.as_str()).collect();
        sqlx::query(
            "INSERT INTO shell_sources (scan_id, source_id, kind, path, task, idx, lines, branches, digest)
             SELECT $1, * FROM UNNEST($2::text[], $3::text[], $4::text[], $5::text[], $6::int[],
                                      $7::int[], $8::int[], $9::text[])
             ON CONFLICT (scan_id, source_id) DO NOTHING",
        )
        .bind(scan.scan_id)
        .bind(&ids)
        .bind(&kinds)
        .bind(&paths)
        .bind(&tasks)
        .bind(&idx)
        .bind(&lines)
        .bind(&branches)
        .bind(&digests)
        .execute(&mut *tx)
        .await?;

        let f = &scan.findings;
        let ordinals: Vec<i32> = (0..f.len())
            .map(|i| i32::try_from(i).unwrap_or(i32::MAX))
            .collect();
        let source_ids: Vec<&str> = f.iter().map(|x| x.source_id.as_str()).collect();
        let line: Vec<i32> = f.iter().map(|x| to_i32(x.line)).collect();
        let col: Vec<i32> = f.iter().map(|x| to_i32(x.column)).collect();
        let end_line: Vec<i32> = f.iter().map(|x| to_i32(x.end_line)).collect();
        let end_col: Vec<i32> = f.iter().map(|x| to_i32(x.end_column)).collect();
        let levels: Vec<Option<String>> = f.iter().map(|x| wire_name(&x.level)).collect();
        let codes: Vec<i32> = f.iter().map(|x| to_i32(x.code)).collect();
        let messages: Vec<&str> = f.iter().map(|x| x.message.as_str()).collect();
        sqlx::query(
            "INSERT INTO shell_findings (scan_id, ordinal, source_id, line, col, end_line, end_col,
                 level, code, message)
             SELECT $1, * FROM UNNEST($2::int[], $3::text[], $4::int[], $5::int[], $6::int[],
                                      $7::int[], $8::text[], $9::int[], $10::text[])
             ON CONFLICT (scan_id, ordinal) DO NOTHING",
        )
        .bind(scan.scan_id)
        .bind(&ordinals)
        .bind(&source_ids)
        .bind(&line)
        .bind(&col)
        .bind(&end_line)
        .bind(&end_col)
        .bind(&levels)
        .bind(&codes)
        .bind(&messages)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    // --- commits -------------------------------------------------------------

    /// Store a classified commit and its contributors. `on_default_branch`
    /// only ever turns on: a commit merged to the default branch stays there.
    pub async fn upsert_commit(
        &self,
        c: &ClassifiedCommit,
        on_default_branch: bool,
    ) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO commits (sha, authored_at, committed_at, author_login, author_name,
                 author_email, author, author_kind, assistants, attribution, message, subject,
                 conv_type, on_default_branch, html_url)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT (sha) DO UPDATE SET
                 authored_at = EXCLUDED.authored_at,
                 committed_at = EXCLUDED.committed_at,
                 author_login = EXCLUDED.author_login,
                 author_name = EXCLUDED.author_name,
                 author_email = EXCLUDED.author_email,
                 author = EXCLUDED.author,
                 author_kind = EXCLUDED.author_kind,
                 assistants = EXCLUDED.assistants,
                 attribution = EXCLUDED.attribution,
                 message = EXCLUDED.message,
                 subject = EXCLUDED.subject,
                 conv_type = EXCLUDED.conv_type,
                 on_default_branch = commits.on_default_branch OR EXCLUDED.on_default_branch,
                 html_url = COALESCE(EXCLUDED.html_url, commits.html_url),
                 synced_at = now()",
        )
        .bind(&c.sha)
        .bind(c.authored_at)
        .bind(c.committed_at)
        .bind(c.author_login.as_deref())
        .bind(&c.author_name)
        .bind(&c.author_email)
        .bind(&c.author)
        .bind(c.author_kind.as_str())
        .bind(&c.assistants)
        .bind(c.attribution.as_str())
        .bind(&c.message)
        .bind(&c.subject)
        .bind(c.conv_type.as_deref())
        .bind(on_default_branch)
        .bind(c.html_url.as_deref())
        .execute(&mut *tx)
        .await?;
        for (id, kind, display_name) in &c.contributors {
            sqlx::query(
                "INSERT INTO contributors (contributor, kind, display_name, first_seen, last_seen)
                 VALUES ($1, $2, $3, $4, $4)
                 ON CONFLICT (contributor) DO UPDATE SET
                     kind = EXCLUDED.kind,
                     display_name = CASE WHEN EXCLUDED.last_seen >= contributors.last_seen
                                         THEN EXCLUDED.display_name ELSE contributors.display_name END,
                     first_seen = LEAST(contributors.first_seen, EXCLUDED.first_seen),
                     last_seen = GREATEST(contributors.last_seen, EXCLUDED.last_seen)",
            )
            .bind(id)
            .bind(kind.as_str())
            .bind(display_name)
            .bind(c.authored_at)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }

    /// Size and touched files from the single-commit endpoint (replaces any
    /// earlier file list).
    pub async fn store_commit_details(
        &self,
        sha: &str,
        additions: i64,
        deletions: i64,
        files: &[CommitFile],
    ) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE commits SET additions = $2, deletions = $3, files = $4, details_synced = true
             WHERE sha = $1",
        )
        .bind(sha)
        .bind(i32::try_from(additions).unwrap_or(i32::MAX))
        .bind(i32::try_from(deletions).unwrap_or(i32::MAX))
        .bind(i32::try_from(files.len()).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM commit_files WHERE sha = $1")
            .bind(sha)
            .execute(&mut *tx)
            .await?;
        let paths: Vec<&str> = files.iter().map(|f| f.filename.as_str()).collect();
        let statuses: Vec<&str> = files.iter().map(|f| f.status.as_str()).collect();
        let adds: Vec<i32> = files
            .iter()
            .map(|f| i32::try_from(f.additions).unwrap_or(i32::MAX))
            .collect();
        let dels: Vec<i32> = files
            .iter()
            .map(|f| i32::try_from(f.deletions).unwrap_or(i32::MAX))
            .collect();
        let previous: Vec<Option<&str>> = files
            .iter()
            .map(|f| f.previous_filename.as_deref())
            .collect();
        sqlx::query(
            "INSERT INTO commit_files (sha, path, status, additions, deletions, previous_path)
             SELECT $1, * FROM UNNEST($2::text[], $3::text[], $4::int[], $5::int[], $6::text[])
             ON CONFLICT (sha, path) DO NOTHING",
        )
        .bind(sha)
        .bind(&paths)
        .bind(&statuses)
        .bind(&adds)
        .bind(&dels)
        .bind(&previous)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }

    /// GitHub no longer has the commit: stop asking.
    pub async fn mark_commit_details_unavailable(&self, sha: &str) -> sqlx::Result<()> {
        sqlx::query("UPDATE commits SET details_synced = true WHERE sha = $1")
            .bind(sha)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn commits_missing_details(&self) -> sqlx::Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT sha FROM commits WHERE NOT details_synced ORDER BY committed_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Run head commits not stored yet, minus those GitHub already said it
    /// does not have (recorded as `commits` sync errors), each with whether a
    /// `push` run on the default branch proves it reached that branch.
    pub async fn run_heads_without_commit(&self) -> sqlx::Result<Vec<(String, bool)>> {
        sqlx::query_as(
            "SELECT r.head_sha,
                    COALESCE(bool_or(r.event = 'push'
                                     AND r.branch = (SELECT default_branch FROM repository LIMIT 1)),
                             false)
             FROM ci_run_attempts r
             WHERE NOT EXISTS (SELECT 1 FROM commits c WHERE c.sha = r.head_sha)
               AND NOT EXISTS (SELECT 1 FROM sync_errors e
                               WHERE e.source = 'commits' AND e.subject = r.head_sha)
             GROUP BY r.head_sha",
        )
        .fetch_all(&self.pool)
        .await
    }

    /// Recompute `reverted` and `fixup_followed` for every commit; returns
    /// how many rows changed.
    ///
    /// - reverted: another commit's message contains `This reverts commit <sha>`.
    /// - fixup_followed: a `fix` commit (any author, not this one) committed
    ///   within 7 days after it touches at least one of its files.
    pub async fn refresh_derived_flags(&self) -> sqlx::Result<u64> {
        let done = sqlx::query(
            "UPDATE commits c SET reverted = x.reverted, fixup_followed = x.fixup_followed
             FROM (
                 SELECT c2.sha,
                        EXISTS (SELECT 1 FROM commits r
                                WHERE r.sha <> c2.sha
                                  AND strpos(r.message, 'This reverts commit ' || c2.sha) > 0) AS reverted,
                        EXISTS (SELECT 1 FROM commits f
                                JOIN commit_files ff ON ff.sha = f.sha
                                JOIN commit_files cf ON cf.sha = c2.sha AND cf.path = ff.path
                                WHERE f.sha <> c2.sha
                                  AND f.conv_type = 'fix'
                                  AND f.committed_at > c2.committed_at
                                  AND f.committed_at <= c2.committed_at + interval '7 days') AS fixup_followed
                 FROM commits c2
             ) x
             WHERE x.sha = c.sha
               AND (c.reverted IS DISTINCT FROM x.reverted
                    OR c.fixup_followed IS DISTINCT FROM x.fixup_followed)",
        )
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }

    // --- taskgraph warehouse -------------------------------------------------

    /// Apply facts in one transaction. `event_id` is the idempotency key: a
    /// fact seen before is skipped, so redeliveries and re-published
    /// artifacts change nothing. Every write is an upsert of the columns its
    /// fact owns, so facts of one run may arrive in any order.
    pub async fn apply_events(
        &self,
        events: &[(&TaskgraphEvent, Option<u64>)],
    ) -> sqlx::Result<Applied> {
        let mut tx = self.pool.begin().await?;
        let mut result = Applied::default();
        for (event, seq) in events {
            let fresh = sqlx::query(
                "INSERT INTO tg_events (event_id, type, run_id, at, stream_seq) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (event_id) DO NOTHING",
            )
            .bind(event.event_id)
            .bind(event.body.kind())
            .bind(event.body.run_id())
            .bind(event.at)
            .bind(seq.map(to_i64))
            .execute(&mut *tx)
            .await?
            .rows_affected()
                == 1;
            if fresh {
                apply_body(&mut tx, event).await?;
                result.applied += 1;
            } else {
                result.duplicates += 1;
            }
        }
        tx.commit().await?;
        Ok(result)
    }

    // --- traces --------------------------------------------------------------

    /// Completed counted attempts whose jobs and artifacts are in, not yet
    /// exported, oldest first.
    pub async fn attempts_pending_trace(&self, limit: i64) -> sqlx::Result<Vec<TraceAttempt>> {
        sqlx::query_as(
            "SELECT r.run_id, r.attempt, r.workflow, r.workflow_path, r.event, r.branch, r.head_sha,
                    r.conclusion, r.created_at, r.started_at, r.completed_at, r.actor, r.html_url,
                    c.author, c.author_kind
             FROM ci_run_attempts r
             LEFT JOIN commits c ON c.sha = r.head_sha
             WHERE r.ci_workflow AND r.status = 'completed' AND r.jobs_synced AND r.artifacts_synced
               AND r.trace_exported_at IS NULL AND r.completed_at IS NOT NULL
             ORDER BY r.created_at, r.run_id, r.attempt
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn trace_jobs(&self, run_id: i64, attempt: i32) -> sqlx::Result<Vec<TraceJob>> {
        sqlx::query_as(
            "SELECT job_id, name, conclusion, created_at, started_at, completed_at, runner, html_url
             FROM ci_jobs WHERE run_id = $1 AND attempt = $2 ORDER BY started_at, job_id",
        )
        .bind(run_id)
        .bind(attempt)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn trace_steps(&self, job_ids: &[i64]) -> sqlx::Result<Vec<TraceStep>> {
        sqlx::query_as(
            "SELECT job_id, number, name, conclusion, started_at, completed_at
             FROM ci_steps WHERE job_id = ANY($1) ORDER BY job_id, number",
        )
        .bind(job_ids)
        .fetch_all(&self.pool)
        .await
    }

    /// taskgraph runs recorded inside a GitHub Actions run attempt.
    pub async fn trace_task_runs(
        &self,
        run_id: i64,
        attempt: i32,
    ) -> sqlx::Result<Vec<TraceTaskRun>> {
        sqlx::query_as(
            "SELECT run_id, target, ci_job, outcome, exit_code, invoker_kind, agent, started_at,
                    finished_at, duration_ms
             FROM task_runs
             WHERE provider = 'github_actions' AND ci_run_id = $1 AND COALESCE(ci_attempt, 1) = $2
             ORDER BY started_at",
        )
        .bind(run_id.to_string())
        .bind(attempt)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn trace_executions(&self, run_ids: &[Uuid]) -> sqlx::Result<Vec<TraceExecution>> {
        sqlx::query_as(
            "SELECT run_id, instance, task, parent, via, outcome, started_at, finished_at,
                    duration_ms, error
             FROM task_executions WHERE run_id = ANY($1) ORDER BY run_id, instance",
        )
        .bind(run_ids)
        .fetch_all(&self.pool)
        .await
    }

    /// Record exported traces; only called once the exporter confirmed them.
    pub async fn mark_traces_exported(&self, exported: &[(i64, i32, String)]) -> sqlx::Result<()> {
        let run_ids: Vec<i64> = exported.iter().map(|e| e.0).collect();
        let attempts: Vec<i32> = exported.iter().map(|e| e.1).collect();
        let trace_ids: Vec<&str> = exported.iter().map(|e| e.2.as_str()).collect();
        sqlx::query(
            "UPDATE ci_run_attempts r SET trace_id = x.trace_id, trace_exported_at = now()
             FROM UNNEST($1::bigint[], $2::int[], $3::text[]) AS x (run_id, attempt, trace_id)
             WHERE r.run_id = x.run_id AND r.attempt = x.attempt",
        )
        .bind(&run_ids)
        .bind(&attempts)
        .bind(&trace_ids)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

async fn apply_body(
    tx: &mut Transaction<'_, Postgres>,
    event: &TaskgraphEvent,
) -> sqlx::Result<()> {
    match &event.body {
        EventBody::GraphPublished { .. } => Ok(()),
        EventBody::RunStarted {
            run_id,
            graph_id,
            target,
            args,
            host,
            user,
            cwd,
            estimate_ms,
            origin,
        } => {
            let ci = origin.ci.as_ref();
            let invoker_kind = origin.invoker.as_ref().map(|i| match i.kind {
                InvokerKind::Human => "human",
                InvokerKind::Agent => "agent",
                InvokerKind::Ci => "ci",
            });
            sqlx::query(
                "INSERT INTO task_runs (run_id, graph_id, target, args, host, username, cwd, estimate_ms,
                     provider, ci_run_id, ci_attempt, ci_job, ci_pipeline, ci_repository, ci_ref, ci_sha,
                     ci_event, ci_actor, ci_run_url, invoker_kind, agent, origin_sha, trace_id, started_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18,
                         $19, $20, $21, $22, $23, $24)
                 ON CONFLICT (run_id) DO UPDATE SET
                     graph_id = EXCLUDED.graph_id,
                     target = EXCLUDED.target,
                     args = EXCLUDED.args,
                     host = EXCLUDED.host,
                     username = EXCLUDED.username,
                     cwd = EXCLUDED.cwd,
                     estimate_ms = EXCLUDED.estimate_ms,
                     provider = EXCLUDED.provider,
                     ci_run_id = EXCLUDED.ci_run_id,
                     ci_attempt = EXCLUDED.ci_attempt,
                     ci_job = EXCLUDED.ci_job,
                     ci_pipeline = EXCLUDED.ci_pipeline,
                     ci_repository = EXCLUDED.ci_repository,
                     ci_ref = EXCLUDED.ci_ref,
                     ci_sha = EXCLUDED.ci_sha,
                     ci_event = EXCLUDED.ci_event,
                     ci_actor = EXCLUDED.ci_actor,
                     ci_run_url = EXCLUDED.ci_run_url,
                     invoker_kind = EXCLUDED.invoker_kind,
                     agent = EXCLUDED.agent,
                     origin_sha = EXCLUDED.origin_sha,
                     trace_id = EXCLUDED.trace_id,
                     started_at = EXCLUDED.started_at",
            )
            .bind(run_id)
            .bind(graph_id)
            .bind(target)
            .bind(args)
            .bind(host)
            .bind(user)
            .bind(cwd)
            .bind(estimate_ms.map(to_i64))
            .bind(ci.map(|c| c.provider.as_str()))
            .bind(ci.map(|c| c.run_id.as_str()))
            .bind(ci.and_then(|c| c.run_attempt).map(to_i32))
            .bind(ci.and_then(|c| c.job.as_deref()))
            .bind(ci.and_then(|c| c.pipeline.as_deref()))
            .bind(ci.and_then(|c| c.repository.as_deref()))
            .bind(ci.and_then(|c| c.git_ref.as_deref()))
            .bind(ci.and_then(|c| c.sha.as_deref()))
            .bind(ci.and_then(|c| c.event.as_deref()))
            .bind(ci.and_then(|c| c.actor.as_deref()))
            .bind(ci.and_then(|c| c.run_url.as_deref()))
            .bind(invoker_kind)
            .bind(origin.invoker.as_ref().and_then(|i| i.agent.as_deref()))
            .bind(origin.sha.as_deref())
            .bind(event.trace.as_ref().map(|t| t.trace_id.as_str()))
            .bind(event.at)
            .execute(&mut **tx)
            .await?;
            Ok(())
        }
        EventBody::RunFinished {
            run_id,
            outcome,
            exit_code,
            duration_ms,
            error,
        } => {
            sqlx::query(
                "INSERT INTO task_runs (run_id, finished_at, outcome, exit_code, duration_ms, error)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (run_id) DO UPDATE SET
                     finished_at = EXCLUDED.finished_at,
                     outcome = EXCLUDED.outcome,
                     exit_code = EXCLUDED.exit_code,
                     duration_ms = EXCLUDED.duration_ms,
                     error = EXCLUDED.error",
            )
            .bind(run_id)
            .bind(event.at)
            .bind(wire_name(outcome))
            .bind(exit_code)
            .bind(to_i64(*duration_ms))
            .bind(error.as_deref())
            .execute(&mut **tx)
            .await?;
            Ok(())
        }
        EventBody::TaskStarted {
            run_id,
            instance,
            task,
            parent,
            via,
        } => {
            sqlx::query(
                "INSERT INTO task_executions (run_id, instance, task, parent, via, started_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (run_id, instance) DO UPDATE SET
                     task = EXCLUDED.task,
                     parent = EXCLUDED.parent,
                     via = EXCLUDED.via,
                     started_at = EXCLUDED.started_at",
            )
            .bind(run_id)
            .bind(to_i32(*instance))
            .bind(task)
            .bind(parent.map(to_i32))
            .bind(wire_name(via))
            .bind(event.at)
            .execute(&mut **tx)
            .await?;
            Ok(())
        }
        EventBody::TaskFinished {
            run_id,
            instance,
            task,
            outcome,
            duration_ms,
            error,
        } => {
            sqlx::query(
                "INSERT INTO task_executions (run_id, instance, task, outcome, duration_ms, error, finished_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (run_id, instance) DO UPDATE SET
                     task = EXCLUDED.task,
                     outcome = EXCLUDED.outcome,
                     duration_ms = EXCLUDED.duration_ms,
                     error = EXCLUDED.error,
                     finished_at = EXCLUDED.finished_at",
            )
            .bind(run_id)
            .bind(to_i32(*instance))
            .bind(task)
            .bind(wire_name(outcome))
            .bind(to_i64(*duration_ms))
            .bind(error.as_deref())
            .bind(event.at)
            .execute(&mut **tx)
            .await?;
            Ok(())
        }
        EventBody::CommandStarted {
            run_id,
            instance,
            task,
            command,
        } => {
            sqlx::query(
                "INSERT INTO task_commands (event_id, run_id, instance, task, command, at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (event_id) DO NOTHING",
            )
            .bind(event.event_id)
            .bind(run_id)
            .bind(to_i32(*instance))
            .bind(task)
            .bind(command)
            .bind(event.at)
            .execute(&mut **tx)
            .await?;
            Ok(())
        }
    }
}
