//! `scorecard` / `kind_scorecard` formulas (Contract 4) on a crafted history.
//!
//! September: alice (3 commits, every metric known), bob (3, legacy
//! attribution, a failed first pass and a fixed-up commit), carol (2, below
//! the sample floor), claude-code (assisted one of alice's commits, which
//! bob reverted), a bot. October: erin with CI evidence and dave with none —
//! dave's missing metrics must take the team value, never count as zero.

#![allow(clippy::unwrap_used)]

mod common;

use chrono::{DateTime, Duration, Utc};
use common::insights_db;
use domain_insights::Store;
use uuid::Uuid;

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

struct C<'a> {
    sha: &'a str,
    author: &'a str,
    kind: &'a str,
    assistants: &'a [&'a str],
    attribution: &'a str,
    authored: &'a str,
    message: &'a str,
    conv_type: Option<&'a str>,
    on_default_branch: bool,
    files: &'a [&'a str],
}

/// A human commit on the default branch.
fn human<'a>(
    sha: &'a str,
    author: &'a str,
    attribution: &'a str,
    authored: &'a str,
    message: &'a str,
    conv_type: Option<&'a str>,
    files: &'a [&'a str],
) -> C<'a> {
    C {
        sha,
        author,
        kind: "human",
        assistants: &[],
        attribution,
        authored,
        message,
        conv_type,
        on_default_branch: true,
        files,
    }
}

fn off_main(c: C<'_>) -> C<'_> {
    C {
        on_default_branch: false,
        ..c
    }
}

async fn commit(store: &Store, c: C<'_>) {
    let assistants: Vec<String> = c.assistants.iter().map(|a| a.to_string()).collect();
    sqlx::query(
        "INSERT INTO commits (sha, authored_at, committed_at, author_name, author_email, author,
             author_kind, assistants, attribution, message, subject, conv_type, on_default_branch,
             additions, deletions, files, details_synced)
         VALUES ($1, $2, $2, $3, $3, $3, $4, $5, $6, $7, split_part($7, E'\\n', 1), $8, $9, 10, 1, $10, true)",
    )
    .bind(c.sha)
    .bind(at(c.authored))
    .bind(c.author)
    .bind(c.kind)
    .bind(&assistants)
    .bind(c.attribution)
    .bind(c.message)
    .bind(c.conv_type)
    .bind(c.on_default_branch)
    .bind(c.files.len() as i32)
    .execute(store.pool())
    .await
    .unwrap();
    for path in c.files {
        sqlx::query("INSERT INTO commit_files (sha, path, status, additions, deletions) VALUES ($1, $2, 'modified', 10, 1)")
            .bind(c.sha)
            .bind(path)
            .execute(store.pool())
            .await
            .unwrap();
    }
}

async fn contributor(store: &Store, id: &str, kind: &str) {
    sqlx::query(
        "INSERT INTO contributors (contributor, kind, display_name, first_seen, last_seen)
         VALUES ($1, $2, $1, now(), now())",
    )
    .bind(id)
    .bind(kind)
    .execute(store.pool())
    .await
    .unwrap();
}

/// A counted `push` run on main for `head_sha`, attempt 1.
async fn run(
    store: &Store,
    run_id: i64,
    head_sha: &str,
    created: &str,
    completed: &str,
    conclusion: &str,
) {
    sqlx::query(
        "INSERT INTO ci_run_attempts (run_id, attempt, workflow_id, workflow, workflow_path, ci_workflow,
             run_number, event, branch, head_sha, status, conclusion, created_at, started_at, updated_at,
             completed_at, html_url, jobs_synced)
         VALUES ($1, 1, 1, 'CI', '.github/workflows/ci-optimized.yml', true, $1, 'push', 'main', $2,
                 'completed', $3, $4, $4, $5, $5, 'https://example.com', true)",
    )
    .bind(run_id)
    .bind(head_sha)
    .bind(conclusion)
    .bind(at(created))
    .bind(at(completed))
    .execute(store.pool())
    .await
    .unwrap();
}

/// A taskgraph run at `sha` with one execution per outcome.
async fn task_run(
    store: &Store,
    provider: Option<&str>,
    sha: &str,
    attempt: i32,
    outcomes: &[&str],
) {
    let run_id = Uuid::now_v7();
    sqlx::query("INSERT INTO task_runs (run_id, provider, ci_run_id, ci_attempt, ci_sha, origin_sha) VALUES ($1, $2, '1', $3, $4, $4)")
        .bind(run_id)
        .bind(provider)
        .bind(attempt)
        .bind(sha)
        .execute(store.pool())
        .await
        .unwrap();
    for (i, outcome) in outcomes.iter().enumerate() {
        sqlx::query("INSERT INTO task_executions (run_id, instance, task, outcome, duration_ms) VALUES ($1, $2, 't', $3, 10)")
            .bind(run_id)
            .bind(i as i32 + 1)
            .bind(outcome)
            .execute(store.pool())
            .await
            .unwrap();
    }
}

async fn fixture(store: &Store) {
    sqlx::query("INSERT INTO repository (full_name, default_branch) VALUES ('o/r', 'main')")
        .execute(store.pool())
        .await
        .unwrap();
    for (id, kind) in [
        ("alice", "human"),
        ("bob", "human"),
        ("carol", "human"),
        ("dave", "human"),
        ("erin", "human"),
        ("zed", "human"),
        ("claude-code", "agent"),
        ("dependabot[bot]", "bot"),
    ] {
        contributor(store, id, kind).await;
    }
    commit(
        store,
        human(
            "a1",
            "alice",
            "enforced",
            "2026-09-02T10:00:00Z",
            "feat: x",
            Some("feat"),
            &["src/x.rs"],
        ),
    )
    .await;
    commit(
        store,
        C {
            assistants: &["claude-code"],
            ..human(
                "a2",
                "alice",
                "enforced",
                "2026-09-03T10:00:00Z",
                "feat: y",
                Some("feat"),
                &["src/y.rs"],
            )
        },
    )
    .await;
    commit(
        store,
        human(
            "a3",
            "alice",
            "enforced",
            "2026-09-04T10:00:00Z",
            "docs: readme",
            Some("docs"),
            &["README.md"],
        ),
    )
    .await;
    commit(
        store,
        human(
            "b1",
            "bob",
            "legacy",
            "2026-09-05T10:00:00Z",
            "feat: z",
            Some("feat"),
            &["src/z.rs"],
        ),
    )
    .await;
    commit(
        store,
        human(
            "b2",
            "bob",
            "legacy",
            "2026-09-06T10:00:00Z",
            "fix: z bug",
            Some("fix"),
            &["src/z.rs"],
        ),
    )
    .await;
    // Off the default branch from here on: no lead time, so October's green
    // runs do not leak into September's medians.
    let revert = "Revert \"feat: y\"\n\nThis reverts commit a2.";
    commit(
        store,
        off_main(human(
            "b3",
            "bob",
            "legacy",
            "2026-09-07T10:00:00Z",
            revert,
            None,
            &["src/y.rs"],
        )),
    )
    .await;
    commit(
        store,
        off_main(human(
            "c1",
            "carol",
            "enforced",
            "2026-09-08T10:00:00Z",
            "chore: c",
            Some("chore"),
            &["c.txt"],
        )),
    )
    .await;
    commit(
        store,
        off_main(human(
            "c2",
            "carol",
            "enforced",
            "2026-09-09T10:00:00Z",
            "chore: c",
            Some("chore"),
            &["c.txt"],
        )),
    )
    .await;
    commit(
        store,
        C {
            kind: "bot",
            ..off_main(human(
                "d1",
                "dependabot[bot]",
                "legacy",
                "2026-09-10T10:00:00Z",
                "chore(deps): bump",
                Some("chore"),
                &["Cargo.lock"],
            ))
        },
    )
    .await;
    // Exactly at the end of September's window: belongs to October only.
    commit(
        store,
        off_main(human(
            "z1",
            "zed",
            "enforced",
            "2026-10-01T00:00:00Z",
            "chore: z",
            Some("chore"),
            &["z.txt"],
        )),
    )
    .await;
    for (sha, day) in [("e1", "02"), ("e2", "03"), ("e3", "04")] {
        let (authored, file) = (format!("2026-10-{day}T10:00:00Z"), format!("{sha}.rs"));
        commit(
            store,
            human(
                sha,
                "erin",
                "enforced",
                &authored,
                "feat: e",
                Some("feat"),
                &[&file],
            ),
        )
        .await;
    }
    for (sha, day) in [("f1", "05"), ("f2", "06"), ("f3", "07")] {
        let (authored, file) = (format!("2026-10-{day}T10:00:00Z"), format!("{sha}.rs"));
        commit(
            store,
            off_main(human(
                sha,
                "dave",
                "enforced",
                &authored,
                "feat: f",
                Some("feat"),
                &[&file],
            )),
        )
        .await;
    }

    // September CI: alice passes everywhere; bob's b1 fails its first run.
    run(
        store,
        1,
        "a1",
        "2026-09-02T10:05:00Z",
        "2026-09-02T11:00:00Z",
        "success",
    )
    .await;
    run(
        store,
        2,
        "a2",
        "2026-09-03T10:05:00Z",
        "2026-09-03T12:00:00Z",
        "success",
    )
    .await;
    run(
        store,
        3,
        "a3",
        "2026-09-04T10:05:00Z",
        "2026-09-04T13:00:00Z",
        "success",
    )
    .await;
    run(
        store,
        4,
        "b1",
        "2026-09-05T10:05:00Z",
        "2026-09-05T10:30:00Z",
        "failure",
    )
    .await;
    run(
        store,
        5,
        "b2",
        "2026-09-06T10:05:00Z",
        "2026-09-06T14:00:00Z",
        "success",
    )
    .await;
    // October CI: erin lands each commit in an hour.
    run(
        store,
        6,
        "e1",
        "2026-10-02T10:05:00Z",
        "2026-10-02T11:00:00Z",
        "success",
    )
    .await;
    run(
        store,
        7,
        "e2",
        "2026-10-03T10:05:00Z",
        "2026-10-03T11:00:00Z",
        "success",
    )
    .await;
    run(
        store,
        8,
        "e3",
        "2026-10-04T10:05:00Z",
        "2026-10-04T11:00:00Z",
        "success",
    )
    .await;

    // CI task executions at attempt 1; up_to_date is neither pass nor fail.
    task_run(
        store,
        Some("github_actions"),
        "a1",
        1,
        &[
            "succeeded",
            "succeeded",
            "succeeded",
            "failed",
            "up_to_date",
        ],
    )
    .await;
    task_run(
        store,
        Some("github_actions"),
        "b1",
        1,
        &["succeeded", "failed"],
    )
    .await;
    // Ignored: a re-run attempt and a local run at the same commits.
    task_run(
        store,
        Some("github_actions"),
        "a1",
        2,
        &["failed", "failed"],
    )
    .await;
    task_run(store, None, "a1", 1, &["failed", "failed"]).await;

    let changed = store.refresh_derived_flags().await.unwrap();
    assert_eq!(changed, 2, "a2 reverted by b3, b1 fixed up by b2");
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    rank: Option<i32>,
    contributor: String,
    kind: String,
    commits: i32,
    authored: i32,
    assisted: i32,
    active_days: i32,
    ci_runs: i32,
    first_pass_rate: Option<f64>,
    change_failure_rate: Option<f64>,
    fixup_rate: Option<f64>,
    lead_time_h: Option<f64>,
    task_failure_rate: Option<f64>,
    outcome_score: Option<f64>,
    sample_ok: bool,
    why: String,
}

async fn scorecard(store: &Store, from: &str, to: &str) -> Vec<Row> {
    sqlx::query_as("SELECT * FROM scorecard($1, $2, 3)")
        .bind(at(from))
        .bind(at(to))
        .fetch_all(store.pool())
        .await
        .unwrap()
}

fn outcome_score(fp: f64, cf: f64, fx: f64, lt: f64, tf: f64) -> f64 {
    100.0
        * (0.30 * fp
            + 0.25 * (1.0 - cf)
            + 0.15 * (1.0 - fx)
            + 0.15 / (1.0 + lt / 24.0)
            + 0.15 * (1.0 - tf))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.006
}

#[tokio::test]
async fn scorecard_ranks_outcomes_and_explains_them() {
    let (_db, store) = insights_db().await;
    fixture(&store).await;

    let rows = scorecard(&store, "2026-09-01T00:00:00Z", "2026-10-01T00:00:00Z").await;
    let get = |id: &str| {
        rows.iter()
            .find(|r| r.contributor == id)
            .unwrap_or_else(|| panic!("{id}"))
    };
    assert!(
        rows.iter().all(|r| r.contributor != "zed"),
        "the window's end is exclusive"
    );

    let alice = get("alice");
    assert_eq!(
        (
            alice.commits,
            alice.authored,
            alice.assisted,
            alice.active_days,
            alice.ci_runs
        ),
        (3, 3, 0, 3, 3)
    );
    assert_eq!(alice.first_pass_rate, Some(1.0));
    assert!(close(alice.change_failure_rate.unwrap(), 1.0 / 3.0));
    assert_eq!(alice.fixup_rate, Some(0.0));
    assert_eq!(alice.lead_time_h, Some(2.0), "median of 1h, 2h, 3h");
    assert_eq!(
        alice.task_failure_rate,
        Some(0.25),
        "1 failed of 4 decided; up_to_date, attempt 2 and local runs excluded"
    );
    assert!(close(
        alice.outcome_score.unwrap(),
        outcome_score(1.0, 1.0 / 3.0, 0.0, 2.0, 0.25)
    ));
    assert_eq!(
        alice.why,
        "strengths: first_pass_rate 100% vs team 80%, fixup_rate 0% vs team 11%; \
         weakest: change_failure_rate 33% vs team 11%"
    );

    let bob = get("bob");
    assert_eq!(bob.first_pass_rate, Some(0.5));
    assert!(close(bob.fixup_rate.unwrap(), 1.0 / 3.0));
    // b1 waits for b2's green run (28h); b2 takes 4h; b3 has no later green run.
    assert_eq!(bob.lead_time_h, Some(16.0));
    assert!(close(
        bob.outcome_score.unwrap(),
        outcome_score(0.5, 0.0, 1.0 / 3.0, 16.0, 0.5)
    ));
    assert_eq!(
        bob.why,
        "strengths: change_failure_rate 0% vs team 11%; weakest: first_pass_rate 50% vs team 80% \
         (legacy attribution: agent share unknown)"
    );

    assert_eq!((alice.rank, bob.rank), (Some(1), Some(2)));

    let carol = get("carol");
    assert!(!carol.sample_ok);
    assert_eq!(carol.rank, None);
    assert_eq!(carol.why, "insufficient data: 2 commits (< 3)");

    // An assistant is credited with the commits it assisted, as an agent.
    let claude = get("claude-code");
    assert_eq!(
        (
            claude.kind.as_str(),
            claude.commits,
            claude.authored,
            claude.assisted
        ),
        ("agent", 1, 0, 1)
    );
    assert_eq!(claude.change_failure_rate, Some(1.0));
    assert_eq!(get("dependabot[bot]").kind, "bot");
}

#[tokio::test]
async fn missing_metrics_take_the_team_value() {
    let (_db, store) = insights_db().await;
    fixture(&store).await;

    let rows = scorecard(&store, "2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z").await;
    let get = |id: &str| {
        rows.iter()
            .find(|r| r.contributor == id)
            .unwrap_or_else(|| panic!("{id}"))
    };
    let (erin, dave) = (get("erin"), get("dave"));

    // No task data for anyone: that term drops out and the rest renormalise.
    let lead_score = 1.0 / (1.0 + 1.0 / 24.0);
    let expected = 100.0 * (0.30 + 0.25 + 0.15 + 0.15 * lead_score) / 0.85;
    assert_eq!(erin.lead_time_h, Some(1.0));
    assert!(
        close(erin.outcome_score.unwrap(), expected),
        "{:?}",
        erin.outcome_score
    );

    // dave has no CI evidence at all: shown as unknown, scored as the team.
    assert_eq!(
        (
            dave.first_pass_rate,
            dave.lead_time_h,
            dave.task_failure_rate
        ),
        (None, None, None)
    );
    assert_eq!(dave.outcome_score, erin.outcome_score);
    assert_eq!((erin.rank, dave.rank), (Some(1), Some(1)));
    assert!(get("zed").rank.is_none());
}

#[tokio::test]
async fn kind_scorecard_partitions_commits_by_authorship() {
    let (_db, store) = insights_db().await;
    fixture(&store).await;

    #[derive(sqlx::FromRow)]
    struct KindRow {
        kind: String,
        contributors: i32,
        commits: i32,
        first_pass_rate: Option<f64>,
        change_failure_rate: Option<f64>,
    }
    let rows: Vec<KindRow> = sqlx::query_as("SELECT * FROM kind_scorecard($1, $2)")
        .bind(at("2026-09-01T00:00:00Z"))
        .bind(at("2026-09-01T00:00:00Z") + Duration::days(30))
        .fetch_all(store.pool())
        .await
        .unwrap();
    let kinds: Vec<(&str, i32, i32)> = rows
        .iter()
        .map(|r| (r.kind.as_str(), r.contributors, r.commits))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("human", 2, 4),       // a1 a3 c1 c2
            ("human+agent", 1, 1), // a2
            ("bot", 1, 1),         // d1
            ("unknown", 1, 3),     // bob: legacy, no trailer
        ]
    );
    let human_agent = &rows[1];
    assert_eq!(
        (human_agent.first_pass_rate, human_agent.change_failure_rate),
        (Some(1.0), Some(1.0))
    );
}
