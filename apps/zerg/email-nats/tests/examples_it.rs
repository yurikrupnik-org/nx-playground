//! Runs the publisher examples as processes and asserts what landed in JetStream.
//!
//! `examples/publish_test.rs` and `examples/publish_bulk.rs` are the entry points
//! `just email-publish-one` and `just email-scale-check` shell out to, so a break
//! here breaks the email work-queue harness in docs/architecture-backlog.md. We
//! boot a throwaway NATS with JetStream, run each example exactly as documented,
//! and check the stream — not just the exit code.
//!
//! Requires Docker. Run: `cargo test -p zerg_email_nats --test examples_it`.

use std::process::Command;

use email::EmailNatsStream;
use messaging::nats::StreamConfig;
use test_utils::TestNats;

/// The cargo that invoked this test, so the examples build with the same
/// toolchain rather than whatever a bare `cargo` on PATH resolves to.
fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

/// Messages currently held by the EMAILS stream.
async fn stream_messages(nats: &TestNats) -> u64 {
    let mut stream = nats
        .jetstream()
        .get_stream(EmailNatsStream::STREAM_NAME)
        .await
        .expect("EMAILS stream should exist — the example is supposed to create it");
    stream.info().await.expect("stream info").state.messages
}

#[tokio::test]
async fn publish_test_example_enqueues_one_job() {
    let nats = TestNats::new().await;

    let output = Command::new(cargo())
        .args([
            "run",
            "-q",
            "-p",
            "zerg_email_nats",
            "--example",
            "publish_test",
        ])
        .env("NATS_URL", nats.connection_string())
        .output()
        .expect("failed to spawn cargo run --example publish_test");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "example exited with {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status,
    );
    assert!(
        stdout.contains("Published!"),
        "example did not report a publish\n--- stdout ---\n{stdout}",
    );

    // Exactly one: `just email-replica-check` counts deliveries against this
    // assumption, and a publisher that emits two would make that gate lie.
    assert_eq!(
        stream_messages(&nats).await,
        1,
        "expected exactly one job on {}",
        EmailNatsStream::STREAM_NAME,
    );
}

#[tokio::test]
async fn publish_bulk_example_enqueues_the_requested_count() {
    const COUNT: u64 = 25;

    let nats = TestNats::new().await;

    let output = Command::new(cargo())
        .args([
            "run",
            "-q",
            "-p",
            "zerg_email_nats",
            "--example",
            "publish_bulk",
            "--",
            &COUNT.to_string(),
        ])
        .env("NATS_URL", nats.connection_string())
        .output()
        .expect("failed to spawn cargo run --example publish_bulk");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "example exited with {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status,
    );
    assert!(
        stdout.contains(&format!("published {COUNT} jobs")),
        "example did not report its throughput line\n--- stdout ---\n{stdout}",
    );

    // The example acks every publish before exiting, so the count is exact rather
    // than eventually-consistent — no polling needed.
    assert_eq!(
        stream_messages(&nats).await,
        COUNT,
        "publisher acked all jobs but the stream disagrees",
    );
}

/// Both examples build the stream through `stream_config_for` rather than
/// hardcoding a config. Hardcoding is what silently recreated EMAILS with the
/// wrong retention and restored per-replica fan-out (architecture-backlog 0.1),
/// so assert the retention a publisher-created stream actually gets.
#[tokio::test]
async fn publisher_created_stream_uses_the_shared_work_queue_config() {
    let nats = TestNats::new().await;

    let status = Command::new(cargo())
        .args([
            "run",
            "-q",
            "-p",
            "zerg_email_nats",
            "--example",
            "publish_test",
        ])
        .env("NATS_URL", nats.connection_string())
        .status()
        .expect("failed to spawn cargo run --example publish_test");
    assert!(status.success(), "example exited with {status}");

    let mut stream = nats
        .jetstream()
        .get_stream(EmailNatsStream::STREAM_NAME)
        .await
        .expect("EMAILS stream");
    let info = stream.info().await.expect("stream info");

    let expected = messaging::nats::stream_config_for(
        EmailNatsStream::STREAM_NAME,
        EmailNatsStream::SUBJECT,
        EmailNatsStream::KIND,
    );
    assert_eq!(
        info.config.retention, expected.retention,
        "publisher created EMAILS with the wrong retention; replicas would fan out",
    );
    assert_eq!(info.config.subjects, expected.subjects);
}
