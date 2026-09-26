//! What reaches the DLQ, and what you can do with it — against a real JetStream server.
//!
//! These cover three defects that were live before 2026-07-26. Each test fails against
//! the old code, which is the point: all three were silent in production and invisible
//! to the existing suite.
//!
//! 1. **The retry counter never incremented.** `handle_error_inner` read
//!    `job.retry_count()` from the *payload*, but `nak` redelivers the stored bytes, so
//!    any counter the consumer bumped was discarded. It stayed 0 forever,
//!    `should_retry` was always true, and the DLQ branch for transient errors was
//!    unreachable. Messages died at JetStream's `max_deliver` with no DLQ entry.
//! 2. **Poison messages were acked and dropped.** An undeserializable payload hit
//!    `message.ack()`, which on a `WorkQueue` stream *deletes* it. A producer/consumer
//!    schema skew ate the backlog leaving only a `warn!`.
//! 3. **The DLQ was write-only.** No subject was recorded, so an entry could not be
//!    routed back even by hand.
//!
//! Requires Docker. Run: `cargo test -p messaging --features nats --test dlq_it`.

#![cfg(feature = "nats")]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_nats::jetstream::consumer::pull::Config as PullConfig;
use futures::StreamExt;
use messaging::nats::{
    DlqEntry, DlqPayload, NatsWorker, StreamKind, WorkerConfig, stream_config_for,
};
use messaging::{Job, ProcessingError, Processor};
use serde::{Deserialize, Serialize};
use test_utils::TestNats;
use tokio::sync::watch;
use uuid::Uuid;

const STREAM: &str = "TEST_DLQ_JOBS";
const DLQ: &str = "TEST_DLQ_JOBS_DLQ";
const SUBJECT_ROOT: &str = "tdlq";
const MAX_DELIVER: i64 = 3;

#[derive(Clone, Serialize, Deserialize)]
struct FlakyJob {
    id: Uuid,
    label: String,
}

impl Job for FlakyJob {
    fn job_id(&self) -> Uuid {
        self.id
    }

    fn job_type(&self) -> &'static str {
        "flaky_job"
    }
}

/// Always fails with the given category. Counts how many times it was called so the
/// test can assert the *server* stopped redelivering, not just that the DLQ filled.
struct AlwaysFails {
    category: &'static str,
    calls: Arc<AtomicU32>,
}

impl Processor<FlakyJob> for AlwaysFails {
    async fn process(&self, _job: &FlakyJob) -> Result<(), ProcessingError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(match self.category {
            "permanent" => ProcessingError::permanent("nope"),
            _ => ProcessingError::transient("downstream is down"),
        })
    }

    fn name(&self) -> &'static str {
        "always_fails"
    }
}

fn worker_config(subject: &str) -> WorkerConfig {
    WorkerConfig {
        stream_name: STREAM.to_string(),
        consumer_name: "dlq-test-worker".to_string(),
        subject: subject.to_string(),
        dlq_stream: DLQ.to_string(),
        batch_size: 10,
        fetch_timeout: Duration::from_secs(2),
        max_deliver: MAX_DELIVER,
        ack_wait: Duration::from_secs(2),
        max_concurrent_jobs: 1,
        kind: StreamKind::JobQueue,
        ..Default::default()
    }
}

/// Run the worker until `deadline` elapses, then shut it down.
async fn run_worker(
    js: async_nats::jetstream::Context,
    config: WorkerConfig,
    processor: AlwaysFails,
    deadline: Duration,
) {
    let worker = NatsWorker::<FlakyJob, _>::new(js, processor, config)
        .await
        .expect("build worker");

    let (tx, rx) = watch::channel(false);
    let handle = tokio::spawn(async move { worker.run(rx).await });

    tokio::time::sleep(deadline).await;
    tx.send(true).expect("signal shutdown");
    let _ = tokio::time::timeout(Duration::from_secs(10), handle).await;
}

/// Read every entry currently in the DLQ stream.
async fn dlq_entries(js: &async_nats::jetstream::Context) -> Vec<DlqEntry> {
    // No DLQ stream at all is the pre-fix failure mode, reported as "0 entries".
    let Ok(stream) = js.get_stream(DLQ).await else {
        return Vec::new();
    };

    let consumer = stream
        .create_consumer(PullConfig {
            durable_name: Some(format!("reader-{}", Uuid::new_v4())),
            ..Default::default()
        })
        .await
        .expect("dlq reader");

    let mut batch = consumer
        .fetch()
        .max_messages(50)
        .expires(Duration::from_secs(3))
        .messages()
        .await
        .expect("fetch dlq");

    let mut out = Vec::new();
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(3), batch.next()).await {
        out.push(serde_json::from_slice(&msg.payload).expect("entry is a DlqEntry"));
        msg.ack().await.expect("ack");
    }
    out
}

/// Bug 1. A job that always fails transiently must land in the DLQ once JetStream's
/// `max_deliver` is spent — and must not be redelivered beyond it.
///
/// Before the fix the payload counter was pinned at 0, so `should_retry` never went
/// false: the worker naked forever, the server gave up at `max_deliver`, and the
/// message vanished with an empty DLQ. This test asserted zero entries.
#[tokio::test]
async fn transient_failure_reaches_the_dlq_after_max_deliver() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();
    let subject = format!("{SUBJECT_ROOT}.transient");

    js.create_stream(stream_config_for(
        STREAM,
        format!("{SUBJECT_ROOT}.>"),
        StreamKind::JobQueue,
    ))
    .await
    .expect("create stream");

    let job = FlakyJob {
        id: Uuid::now_v7(),
        label: "transient".to_string(),
    };
    js.publish(subject.clone(), serde_json::to_vec(&job).unwrap().into())
        .await
        .expect("publish")
        .await
        .expect("publish ack");

    let calls = Arc::new(AtomicU32::new(0));
    run_worker(
        js.clone(),
        worker_config(&format!("{SUBJECT_ROOT}.>")),
        AlwaysFails {
            category: "transient",
            calls: calls.clone(),
        },
        Duration::from_secs(20),
    )
    .await;

    let entries = dlq_entries(&js).await;
    assert_eq!(
        entries.len(),
        1,
        "a permanently-failing transient job must reach the DLQ, got {} entries after \
         {} processing attempts",
        entries.len(),
        calls.load(Ordering::SeqCst)
    );

    let entry = &entries[0];
    assert_eq!(entry.job_id, Some(job.id));
    assert_eq!(entry.job_type, "flaky_job");
    assert_eq!(
        entry.original_subject, subject,
        "the original subject must be recorded or the entry cannot be redriven"
    );
    assert!(
        entry.delivery_count >= 2,
        "the entry should record the server's attempt count, got {}",
        entry.delivery_count
    );
    assert!(
        matches!(entry.payload, DlqPayload::Job(_)),
        "a decoded job must be stored as DlqPayload::Job so it can be redriven"
    );

    // The server must have stopped: no delivery beyond max_deliver.
    let attempts = calls.load(Ordering::SeqCst);
    assert!(
        attempts <= MAX_DELIVER as u32,
        "worker was called {attempts} times, more than max_deliver={MAX_DELIVER}"
    );
}

/// Bug 2. An undeserializable payload must be captured, not silently deleted.
///
/// Before the fix `fetch` called `ack()` on it, which deletes the message on a
/// `WorkQueue` stream — the bytes were unrecoverable and the DLQ stayed empty.
#[tokio::test]
async fn poison_message_is_captured_not_dropped() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();
    let subject = format!("{SUBJECT_ROOT}.poison");

    js.create_stream(stream_config_for(
        STREAM,
        format!("{SUBJECT_ROOT}.>"),
        StreamKind::JobQueue,
    ))
    .await
    .expect("create stream");

    // Valid JSON, wrong shape — exactly what a schema skew produces.
    let garbage = br#"{"id":"not-a-uuid","unexpected":true}"#;
    js.publish(subject.clone(), garbage.to_vec().into())
        .await
        .expect("publish")
        .await
        .expect("publish ack");

    let calls = Arc::new(AtomicU32::new(0));
    run_worker(
        js.clone(),
        worker_config(&format!("{SUBJECT_ROOT}.>")),
        AlwaysFails {
            category: "transient",
            calls: calls.clone(),
        },
        Duration::from_secs(8),
    )
    .await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a poison message must never reach the processor"
    );

    let entries = dlq_entries(&js).await;
    assert_eq!(
        entries.len(),
        1,
        "poison message must be captured in the DLQ"
    );

    let entry = &entries[0];
    assert_eq!(
        entry.job_id, None,
        "poison entries have no decodable job id"
    );
    assert_eq!(entry.original_subject, subject);

    match &entry.payload {
        DlqPayload::Raw { base64 } => {
            use base64::Engine as _;
            use base64::engine::general_purpose::STANDARD;
            let decoded = STANDARD.decode(base64).expect("valid base64");
            assert_eq!(
                decoded, garbage,
                "the original bytes must survive verbatim — they are the only evidence \
                 of what the producer actually sent"
            );
        }
        DlqPayload::Job(_) => panic!("undecodable bytes must not be stored as a decoded job"),
    }
}

/// Bug 3. Redrive republishes a decoded entry to the subject it came from, and refuses
/// to replay poison (which would only re-poison the stream).
#[tokio::test]
async fn redrive_returns_jobs_to_their_original_subject_and_skips_poison() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();
    let subject = format!("{SUBJECT_ROOT}.redrive");

    js.create_stream(stream_config_for(
        STREAM,
        format!("{SUBJECT_ROOT}.>"),
        StreamKind::JobQueue,
    ))
    .await
    .expect("create stream");

    let dlq = messaging::nats::DlqManager::new(Arc::new(js.clone()), DLQ);
    dlq.ensure_stream().await.expect("create dlq");

    let job = FlakyJob {
        id: Uuid::now_v7(),
        label: "redrive-me".to_string(),
    };
    let first = dlq
        .move_to_dlq(&job, &subject, "downstream was down", 1, 3)
        .await
        .expect("dlq the job");
    dlq.move_poison_to_dlq(b"\xff\xfe not json", &subject, "invalid utf-8", 2, 3)
        .await
        .expect("dlq the poison");

    let report = dlq.redrive(first, 10).await.expect("redrive");

    assert_eq!(report.republished, 1, "the decoded job must be republished");
    assert_eq!(
        report.skipped, 1,
        "poison must be skipped: replaying it would re-poison the stream"
    );

    // The republished job is back on the source stream, on its original subject.
    let stream = js.get_stream(STREAM).await.expect("source stream");
    let consumer = stream
        .create_consumer(PullConfig {
            durable_name: Some("redrive-check".to_string()),
            ..Default::default()
        })
        .await
        .expect("consumer");

    let mut batch = consumer
        .fetch()
        .max_messages(10)
        .expires(Duration::from_secs(3))
        .messages()
        .await
        .expect("fetch");

    let mut seen = Vec::new();
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(3), batch.next()).await {
        let decoded: FlakyJob = serde_json::from_slice(&msg.payload).expect("redriven job decodes");
        seen.push((msg.subject.to_string(), decoded.id));
        msg.ack().await.expect("ack");
    }

    assert_eq!(
        seen,
        vec![(subject, job.id)],
        "exactly the decoded job should be back, on the subject it originally used"
    );

    // Redrive must not consume the DLQ: the audit trail outlives the replay.
    let stats = dlq.stats().await.expect("dlq stats");
    assert_eq!(
        stats.total_messages, 2,
        "redrive must leave DLQ entries in place for the audit trail"
    );
}
