//! Re-running a sync must not duplicate anything: every writer is keyed by a
//! natural id and applying the same input twice converges on the same rows.

#![allow(clippy::unwrap_used)]

mod common;

use chrono::{DateTime, Utc};
use common::{count, insights_db};
use contract_taskgraph::shell::{
    FindingLevel, ShellFinding, ShellScan, ShellSource, ShellSourceKind,
};
use contract_taskgraph::{
    CiContext, EventBody, Invoker, InvokerKind, RunOrigin, RunOutcome, TaskOutcome, TaskgraphEvent,
    Via,
};
use domain_insights::classify::classify;
use domain_insights::github::{Commit, Job, WorkflowRun};
use domain_insights::store::{Applied, ArtifactLedger};
use serde_json::json;
use uuid::Uuid;

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn run() -> WorkflowRun {
    serde_json::from_value(json!({
        "id": 36338703831_i64, "name": "CI Optimized", "head_branch": "main",
        "head_sha": "5f17449ced5360c420e4d7c31c6703fde04e14d3",
        "path": ".github/workflows/ci-optimized.yml", "run_number": 40, "event": "push",
        "status": "completed", "conclusion": "success", "workflow_id": 1,
        "html_url": "https://github.com/o/r/actions/runs/36338703831",
        "created_at": "2026-09-20T10:00:00Z", "updated_at": "2026-09-20T10:30:00Z",
        "run_attempt": 1, "run_started_at": "2026-09-20T10:00:02Z",
        "actor": {"login": "dev"}, "triggering_actor": {"login": "dev"}
    }))
    .unwrap()
}

fn jobs() -> Vec<Job> {
    serde_json::from_value(json!([{
        "id": 7001, "run_id": 36338703831_i64, "name": "rust", "status": "completed",
        "conclusion": "success", "created_at": "2026-09-20T10:00:03Z",
        "started_at": "2026-09-20T10:00:20Z", "completed_at": "2026-09-20T10:25:00Z",
        "runner_name": "GitHub Actions 2", "labels": ["ubuntu-latest"], "html_url": null,
        "steps": [
            {"name": "checkout", "status": "completed", "conclusion": "success", "number": 1,
             "started_at": "2026-09-20T10:00:21Z", "completed_at": "2026-09-20T10:00:25Z"},
            {"name": "task check", "status": "completed", "conclusion": "success", "number": 2,
             "started_at": "2026-09-20T10:00:26Z", "completed_at": "2026-09-20T10:24:50Z"}
        ]
    }]))
    .unwrap()
}

fn commit() -> Commit {
    serde_json::from_value(json!({
        "sha": "5f17449ced5360c420e4d7c31c6703fde04e14d3",
        "commit": {
            "author": {"name": "Dev", "email": "dev@example.com", "date": "2026-09-20T09:00:00Z"},
            "committer": {"name": "Dev", "email": "dev@example.com", "date": "2026-09-20T09:55:00Z"},
            "message": "fix(ci): retry flaky step\n\nAssisted-by: claude-code:opus"
        },
        "author": {"login": "dev"},
        "html_url": "https://github.com/o/r/commit/5f17449",
        "stats": {"additions": 10, "deletions": 2},
        "files": [
            {"filename": "a.sh", "status": "modified", "additions": 7, "deletions": 2},
            {"filename": "b.sh", "status": "added", "additions": 3, "deletions": 0}
        ]
    }))
    .unwrap()
}

fn events() -> Vec<TaskgraphEvent> {
    let run_id = Uuid::now_v7();
    let t = |s: &str| at(s);
    vec![
        TaskgraphEvent::new(
            EventBody::RunStarted {
                run_id,
                graph_id: "g".into(),
                target: "check".into(),
                args: vec![],
                host: "runner".into(),
                user: "runner".into(),
                cwd: "/w".into(),
                estimate_ms: Some(1000),
                origin: RunOrigin {
                    ci: Some(CiContext {
                        provider: "github_actions".into(),
                        run_id: "36338703831".into(),
                        run_attempt: Some(1),
                        job: Some("rust".into()),
                        sha: Some("5f17449ced5360c420e4d7c31c6703fde04e14d3".into()),
                        ..CiContext::default()
                    }),
                    invoker: Some(Invoker {
                        kind: InvokerKind::Ci,
                        agent: None,
                    }),
                    sha: None,
                },
            },
            t("2026-09-20T10:00:30Z"),
            None,
        ),
        TaskgraphEvent::new(
            EventBody::TaskStarted {
                run_id,
                instance: 1,
                task: "check".into(),
                parent: None,
                via: Via::Root,
            },
            t("2026-09-20T10:00:31Z"),
            None,
        ),
        TaskgraphEvent::new(
            EventBody::CommandStarted {
                run_id,
                instance: 1,
                task: "check".into(),
                command: "cargo clippy".into(),
            },
            t("2026-09-20T10:00:32Z"),
            None,
        ),
        TaskgraphEvent::new(
            EventBody::TaskFinished {
                run_id,
                instance: 1,
                task: "check".into(),
                outcome: TaskOutcome::UpToDate,
                duration_ms: 1500,
                error: None,
            },
            t("2026-09-20T10:00:33Z"),
            None,
        ),
        TaskgraphEvent::new(
            EventBody::RunFinished {
                run_id,
                outcome: RunOutcome::Succeeded,
                exit_code: Some(0),
                duration_ms: 3000,
                error: None,
            },
            t("2026-09-20T10:00:34Z"),
            None,
        ),
    ]
}

fn scan() -> ShellScan {
    ShellScan {
        scan_id: Uuid::now_v7(),
        scanned_at: at("2026-09-21T00:00:00Z"),
        sha: Some("5f17449".into()),
        tool: "shellcheck 0.10.0".into(),
        ci: None,
        sources: vec![ShellSource {
            id: "file:a.sh".into(),
            kind: ShellSourceKind::File,
            path: "a.sh".into(),
            task: None,
            index: None,
            lines: 12,
            branches: 3,
            digest: "d".into(),
        }],
        findings: vec![ShellFinding {
            source_id: "file:a.sh".into(),
            line: 3,
            column: 1,
            end_line: 3,
            end_column: 9,
            level: FindingLevel::Warning,
            code: 2086,
            message: "Double quote to prevent globbing and word splitting.".into(),
        }],
    }
}

#[tokio::test]
async fn syncing_the_same_input_twice_changes_nothing() {
    let (_db, store) = insights_db().await;
    let registry = core_authorship::registry();
    let (run, jobs, commit, events, scan) = (run(), jobs(), commit(), events(), scan());
    let batch: Vec<(&TaskgraphEvent, Option<u64>)> = events.iter().map(|e| (e, None)).collect();
    // Facts of one run may arrive in any order (artifact vs live publish).
    let reversed: Vec<(&TaskgraphEvent, Option<u64>)> = batch.iter().rev().copied().collect();

    let mut first_apply = Applied::default();
    for pass in 0..2 {
        store.upsert_run_attempt(&run, 1, true).await.unwrap();
        store.store_jobs(run.id, 1, &jobs).await.unwrap();
        store
            .upsert_commit(&classify(registry, &commit), true)
            .await
            .unwrap();
        store
            .store_commit_details(&commit.sha, 10, 2, commit.files.as_deref().unwrap())
            .await
            .unwrap();
        let applied = store
            .apply_events(if pass == 0 { &reversed } else { &batch })
            .await
            .unwrap();
        if pass == 0 {
            first_apply = applied;
        } else {
            assert_eq!(
                applied,
                Applied {
                    applied: 0,
                    duplicates: 5
                },
                "second pass is a no-op"
            );
        }
        let stored = store.store_shell_scan(&scan, Some(1)).await.unwrap();
        assert_eq!(stored, pass == 0, "a scan is stored once");
        store
            .ledger_artifact(&ArtifactLedger {
                artifact_id: 99,
                run_id: run.id,
                name: "taskgraph-events-rust-1",
                kind: "events",
                items: 5,
                malformed: 0,
                error: None,
            })
            .await
            .unwrap();

        for (table, rows) in [
            ("ci_run_attempts", 1),
            ("ci_jobs", 1),
            ("ci_steps", 2),
            ("commits", 1),
            ("commit_files", 2),
            ("contributors", 2),
            ("tg_events", 5),
            ("task_runs", 1),
            ("task_executions", 1),
            ("task_commands", 1),
            ("shell_scans", 1),
            ("shell_sources", 1),
            ("shell_findings", 1),
            ("ingested_artifacts", 1),
        ] {
            assert_eq!(
                count(&store, table).await,
                rows,
                "{table} after pass {pass}"
            );
        }
    }
    assert_eq!(
        first_apply,
        Applied {
            applied: 5,
            duplicates: 0
        }
    );

    // Out-of-order application still assembled the whole run.
    let (outcome, exit_code, provider, ci_job, sha, source): (
        String,
        i32,
        String,
        String,
        String,
        String,
    ) = sqlx::query_as("SELECT outcome, exit_code, provider, ci_job, sha, source FROM task_runs_v")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(
        (
            outcome.as_str(),
            exit_code,
            provider.as_str(),
            ci_job.as_str(),
            source.as_str()
        ),
        ("succeeded", 0, "github_actions", "rust", "ci")
    );
    assert_eq!(sha, commit.sha);
    let (task, outcome, via): (String, String, String) =
        sqlx::query_as("SELECT task, outcome, via FROM task_executions_v")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(
        (task.as_str(), outcome.as_str(), via.as_str()),
        ("check", "up_to_date", "root")
    );

    // The attempt completes with its last job; the head commit names its author.
    let (completed_at, author, assistants): (DateTime<Utc>, String, Vec<String>) = sqlx::query_as(
        "SELECT r.completed_at, r.author, c.assistants FROM ci_runs_v r JOIN commits_v c ON c.sha = r.head_sha",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(completed_at, at("2026-09-20T10:25:00Z"));
    assert_eq!(author, "dev");
    assert_eq!(assistants, vec!["claude-code".to_string()]);
}
