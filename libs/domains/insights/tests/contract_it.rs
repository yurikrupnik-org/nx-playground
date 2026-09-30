//! Grafana dashboards query only these views and functions (Contract 4 in
//! docs/ci-insights.md): a renamed or retyped column breaks a dashboard
//! silently, so the migrations must produce exactly this surface.

#![allow(clippy::unwrap_used)]

mod common;

use common::insights_db;

const TS: &str = "timestamp with time zone";

fn cols(spec: &[(&str, &str)]) -> Vec<(String, String)> {
    spec.iter()
        .map(|(n, t)| (n.to_string(), t.to_string()))
        .collect()
}

#[tokio::test]
async fn views_and_functions_match_the_contract() {
    let (_db, store) = insights_db().await;
    let (b, i, t, f, bool_, u) = (
        "bigint",
        "integer",
        "text",
        "double precision",
        "boolean",
        "uuid",
    );
    let views: Vec<(&str, Vec<(String, String)>)> = vec![
        (
            "ci_runs_v",
            cols(&[
                ("run_id", b),
                ("attempt", i),
                ("is_latest", bool_),
                ("workflow", t),
                ("workflow_path", t),
                ("event", t),
                ("branch", t),
                ("head_sha", t),
                ("status", t),
                ("conclusion", t),
                ("created_at", TS),
                ("started_at", TS),
                ("completed_at", TS),
                ("duration_s", f),
                ("queue_s", f),
                ("actor", t),
                ("html_url", t),
                ("author", t),
                ("author_kind", t),
                ("trace_id", t),
                ("ci_workflow", bool_),
            ]),
        ),
        (
            "ci_jobs_v",
            cols(&[
                ("run_id", b),
                ("attempt", i),
                ("job_id", b),
                ("workflow", t),
                ("job", t),
                ("status", t),
                ("conclusion", t),
                ("started_at", TS),
                ("completed_at", TS),
                ("duration_s", f),
                ("queue_s", f),
                ("runner", t),
                ("html_url", t),
            ]),
        ),
        (
            "ci_steps_v",
            cols(&[
                ("run_id", b),
                ("attempt", i),
                ("job_id", b),
                ("workflow", t),
                ("job", t),
                ("step", t),
                ("number", i),
                ("conclusion", t),
                ("started_at", TS),
                ("completed_at", TS),
                ("duration_s", f),
            ]),
        ),
        (
            "task_runs_v",
            cols(&[
                ("run_id", u),
                ("source", t),
                ("provider", t),
                ("ci_run_id", t),
                ("ci_attempt", i),
                ("ci_job", t),
                ("host", t),
                ("username", t),
                ("invoker_kind", t),
                ("agent", t),
                ("target", t),
                ("outcome", t),
                ("exit_code", i),
                ("started_at", TS),
                ("finished_at", TS),
                ("duration_ms", b),
                ("sha", t),
            ]),
        ),
        (
            "task_executions_v",
            cols(&[
                ("run_id", u),
                ("source", t),
                ("provider", t),
                ("ci_run_id", t),
                ("ci_attempt", i),
                ("ci_job", t),
                ("invoker_kind", t),
                ("agent", t),
                ("target", t),
                ("task", t),
                ("instance", i),
                ("parent", i),
                ("via", t),
                ("outcome", t),
                ("started_at", TS),
                ("finished_at", TS),
                ("duration_ms", b),
                ("self_ms", b),
                ("error", t),
                ("sha", t),
            ]),
        ),
        (
            "commits_v",
            cols(&[
                ("sha", t),
                ("authored_at", TS),
                ("committed_at", TS),
                ("author", t),
                ("author_kind", t),
                ("author_agent", t),
                ("assistants", "ARRAY"),
                ("attribution", t),
                ("on_default_branch", bool_),
                ("subject", t),
                ("conv_type", t),
                ("additions", i),
                ("deletions", i),
                ("files", i),
                ("reverted", bool_),
                ("fixup_followed", bool_),
                ("first_ci_conclusion", t),
                ("lead_time_h", f),
            ]),
        ),
        (
            "contributors_v",
            cols(&[
                ("contributor", t),
                ("kind", t),
                ("display_name", t),
                ("first_seen", TS),
                ("last_seen", TS),
            ]),
        ),
        (
            "shell_scans_v",
            cols(&[
                ("scan_id", u),
                ("scanned_at", TS),
                ("sha", t),
                ("origin", t),
                ("sources", i),
                ("lines", i),
                ("branches", i),
                ("findings", i),
                ("errors", i),
                ("warnings", i),
                ("infos", i),
                ("styles", i),
            ]),
        ),
        (
            "shell_sources_v",
            cols(&[
                ("scan_id", u),
                ("scanned_at", TS),
                ("sha", t),
                ("source_id", t),
                ("kind", t),
                ("path", t),
                ("task", t),
                ("lines", i),
                ("branches", i),
                ("findings", i),
                ("owner", t),
                ("owner_kind", t),
            ]),
        ),
        (
            "shell_findings_v",
            cols(&[
                ("scan_id", u),
                ("scanned_at", TS),
                ("sha", t),
                ("source_id", t),
                ("kind", t),
                ("path", t),
                ("task", t),
                ("line", i),
                ("level", t),
                ("code", i),
                ("message", t),
                ("owner", t),
            ]),
        ),
        (
            "sync_status_v",
            cols(&[
                ("source", t),
                ("last_success_at", TS),
                ("last_error", t),
                ("last_error_at", TS),
                ("items", b),
            ]),
        ),
    ];
    for (view, expected) in views {
        let actual: Vec<(String, String)> = sqlx::query_as(
            "SELECT column_name::text, data_type::text FROM information_schema.columns
             WHERE table_name = $1 ORDER BY ordinal_position",
        )
        .bind(view)
        .fetch_all(store.pool())
        .await
        .unwrap();
        assert_eq!(actual, expected, "{view}");
    }

    let functions: Vec<(String, String)> = sqlx::query_as(
        "SELECT proname::text, pg_get_function_result(oid) FROM pg_proc
         WHERE proname IN ('scorecard', 'kind_scorecard') ORDER BY proname",
    )
    .fetch_all(store.pool())
    .await
    .unwrap();
    assert_eq!(
        functions,
        vec![
            (
                "kind_scorecard".into(),
                "TABLE(kind text, contributors integer, commits integer, additions integer, \
                 deletions integer, first_pass_rate double precision, change_failure_rate double precision, \
                 fixup_rate double precision, lead_time_h double precision, task_failure_rate double precision, \
                 outcome_score double precision)"
                    .into()
            ),
            (
                "scorecard".into(),
                "TABLE(rank integer, contributor text, kind text, commits integer, authored integer, \
                 assisted integer, additions integer, deletions integer, active_days integer, ci_runs integer, \
                 first_pass_rate double precision, change_failure_rate double precision, fixup_rate double precision, \
                 lead_time_h double precision, task_failure_rate double precision, outcome_score double precision, \
                 sample_ok boolean, why text)"
                    .into()
            ),
        ]
    );
}
