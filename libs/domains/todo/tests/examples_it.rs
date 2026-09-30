//! Runs `examples/todo_crud.rs` as a process and asserts what it did.
//!
//! The example is documentation people copy-paste, so the thing under test is the
//! command itself — not a re-implementation of it. We boot a throwaway Postgres,
//! point `DATABASE_URL` at it, run the example exactly as the doc comment says to,
//! then assert both its output and the database state it left behind.
//!
//! Requires Docker. Run: `cargo test -p domain_todo --test examples_it`.

use std::process::Command;

use domain_todo::models::TodoFilter;
use domain_todo::{PgTodoRepository, TodoRepository};
use test_utils::TestDatabase;

/// The cargo that invoked this test, so the example is built with the same
/// toolchain rather than whatever a bare `cargo` on PATH resolves to.
fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

#[tokio::test]
async fn todo_crud_example_runs_against_postgres() {
    let db = TestDatabase::with_migrations_dir("manifests/db/todo/migrations").await;

    let output = Command::new(cargo())
        .args(["run", "-q", "-p", "domain_todo", "--example", "todo_crud"])
        .env("DATABASE_URL", &db.connection_string)
        .output()
        .expect("failed to spawn cargo run --example todo_crud");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "example exited with {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status,
    );

    // Every lifecycle step must have reported. Asserting on the whole set (rather
    // than just the final line) is what makes a silently skipped step fail here.
    for marker in [
        "step:created",
        "step:read",
        "step:updated title=buy oat milk",
        "step:completed completed=true",
        "step:listed completed_count=1",
        "step:counted total=1",
        "step:deleted",
        "step:verified_absent",
    ] {
        assert!(
            stdout.contains(marker),
            "example output missing {marker:?}\n--- stdout ---\n{stdout}",
        );
    }

    // The example claims it cleaned up. Verify against the database rather than
    // trusting its own last line — that is the half a process-level test can check
    // and a doc comment cannot.
    let repo = PgTodoRepository::new(db.connection());
    assert_eq!(
        repo.count().await.expect("count"),
        0,
        "example left rows behind; it is supposed to delete what it creates",
    );
    let remaining = repo
        .list(TodoFilter {
            limit: 50,
            ..Default::default()
        })
        .await
        .expect("list");
    assert!(remaining.is_empty(), "unexpected rows: {remaining:?}");
}

/// The example must fail loudly on an unreachable database, not exit 0 having
/// done nothing. A green run against a broken URL would make the test above
/// meaningless.
#[tokio::test]
async fn todo_crud_example_fails_without_a_database() {
    let output = Command::new(cargo())
        .args(["run", "-q", "-p", "domain_todo", "--example", "todo_crud"])
        // Port 1 is reserved and never listening.
        .env("DATABASE_URL", "postgres://nobody@127.0.0.1:1/nothing")
        .output()
        .expect("failed to spawn cargo run --example todo_crud");

    assert!(
        !output.status.success(),
        "example reported success against an unreachable database\n--- stdout ---\n{}",
        String::from_utf8_lossy(&output.stdout),
    );
}
