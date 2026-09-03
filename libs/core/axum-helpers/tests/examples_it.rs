//! Runs `examples/http_service.rs` as a process and asserts what it did.
//!
//! The example is documentation people copy-paste, so the thing under test is
//! the command itself — not a re-implementation of it. Unlike the todo and
//! email-nats example tests, no container is needed: the example binds an
//! ephemeral localhost port and talks to itself.
//!
//! Run: `cargo test -p axum-helpers --test examples_it`.

use std::process::Command;

/// The cargo that invoked this test, so the example is built with the same
/// toolchain rather than whatever a bare `cargo` on PATH resolves to.
fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

#[test]
fn http_service_example_exercises_the_documented_surface() {
    let output = Command::new(cargo())
        .args([
            "run",
            "-q",
            "-p",
            "axum-helpers",
            "--example",
            "http_service",
        ])
        // The exact invocation the example's doc comment tells people to run.
        .env("CORS_ALLOWED_ORIGIN", "http://localhost:3000")
        .output()
        .expect("failed to spawn cargo run --example http_service");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "example exited with {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status,
    );

    // Every behavior the example claims to demonstrate, in order of appearance.
    // `step:` markers are the contract with examples/http_service.rs.
    for marker in [
        "step:listening",
        "step:health status=200",
        "step:created status=201",
        "step:rejected status=400 error=VALIDATION_ERROR",
        "step:bad_uuid status=400",
        "step:not_found status=404 error=NOT_FOUND",
        "step:fallback status=404",
        "HTTP walkthrough complete",
    ] {
        assert!(
            stdout.contains(marker),
            "example output missing {marker:?}\n--- stdout ---\n{stdout}",
        );
    }
}

/// CORS_ALLOWED_ORIGIN is documented as required — the example must fail loudly
/// without it, not serve with silently-wrong CORS. A green run here would make
/// the doc comment (and create_router's contract) a lie.
#[test]
fn http_service_example_fails_without_cors_config() {
    let output = Command::new(cargo())
        .args([
            "run",
            "-q",
            "-p",
            "axum-helpers",
            "--example",
            "http_service",
        ])
        .env_remove("CORS_ALLOWED_ORIGIN")
        .output()
        .expect("failed to spawn cargo run --example http_service");

    assert!(
        !output.status.success(),
        "example reported success without CORS_ALLOWED_ORIGIN\n--- stdout ---\n{}",
        String::from_utf8_lossy(&output.stdout),
    );
}
