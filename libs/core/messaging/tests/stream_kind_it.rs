//! What `StreamKind` actually buys you, asserted against a real JetStream server.
//!
//! The unit tests in `nats::config` cover the *client* half — every replica derives the
//! same consumer group name. These cover the *server* half, which is the part that makes
//! the guarantee structural instead of conventional:
//!
//! - `JobQueue` -> NATS **refuses** a second overlapping consumer group, so a job stream
//!   cannot be fanned out by a future developer who forgets the convention.
//! - `EventLog` -> independent consumer groups each receive **every** message, which is
//!   the property that lets a second subscriber (analytics, search indexing, audit) be
//!   added later without touching the producer.
//!
//! Neither is exercised anywhere else: `EventLog` is declared on `TODOS` but has only
//! ever had one consumer group, so its defining behaviour was untested until this file.
//! An earlier revision mapped `EventLog` to `Interest` retention, which silently drops
//! messages published while no consumer exists — the kind of defect that only shows up
//! against a real server.
//!
//! Requires Docker. Run: `cargo test -p messaging --features nats --test stream_kind_it`.

#![cfg(feature = "nats")]

use std::time::Duration;

use async_nats::jetstream::consumer::pull::Config as PullConfig;
use futures::StreamExt;
use messaging::nats::{stream_config_for, StreamKind};
use test_utils::TestNats;

/// Create a durable pull consumer representing one consumer group.
async fn add_group(
    stream: &async_nats::jetstream::stream::Stream,
    name: &str,
    filter: &str,
) -> Result<async_nats::jetstream::consumer::Consumer<PullConfig>, async_nats::Error> {
    stream
        .create_consumer(PullConfig {
            durable_name: Some(name.to_string()),
            filter_subject: filter.to_string(),
            ..Default::default()
        })
        .await
        .map_err(Into::into)
}

/// Fetch up to `want` messages, acking each, and return how many arrived.
async fn drain(
    consumer: &async_nats::jetstream::consumer::Consumer<PullConfig>,
    want: usize,
) -> usize {
    let mut batch = consumer
        .fetch()
        .max_messages(want)
        .expires(Duration::from_secs(5))
        .messages()
        .await
        .expect("fetch");

    let mut seen = 0;
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(5), batch.next()).await {
        msg.ack().await.expect("ack");
        seen += 1;
    }
    seen
}

/// The reason `EventLog` exists: a second consumer group can be added at any time and
/// receives every message, rather than competing for them with the first.
#[tokio::test]
async fn event_log_delivers_every_message_to_each_consumer_group() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();

    js.create_stream(stream_config_for(
        "TEST_EVENTS",
        "testev.>",
        StreamKind::EventLog,
    ))
    .await
    .expect("create event log stream");

    let stream = js.get_stream("TEST_EVENTS").await.expect("get stream");

    // Both groups exist before publishing. `Limits` retention would also serve a group
    // registered afterwards; `Interest` would not, which is why it is not used.
    let analytics = add_group(&stream, "analytics", "testev.>")
        .await
        .expect("first group");
    let search = add_group(&stream, "search", "testev.>")
        .await
        .expect("second group on an event log must be allowed");

    const N: usize = 5;
    for i in 0..N {
        js.publish("testev.created", format!("event-{i}").into())
            .await
            .expect("publish")
            .await
            .expect("ack");
    }

    assert_eq!(
        drain(&analytics, N).await,
        N,
        "analytics group missed events"
    );
    assert_eq!(drain(&search, N).await, N, "search group missed events");
}

/// The reason `JobQueue` exists: the server itself forbids the second group, so the
/// duplicate-processing bug is unrepresentable rather than merely discouraged.
#[tokio::test]
async fn job_queue_refuses_a_second_consumer_group() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();

    js.create_stream(stream_config_for(
        "TEST_JOBS",
        "testjob.>",
        StreamKind::JobQueue,
    ))
    .await
    .expect("create job queue stream");

    let stream = js.get_stream("TEST_JOBS").await.expect("get stream");

    add_group(&stream, "worker", "testjob.>")
        .await
        .expect("the one consumer group must be allowed");

    let second = add_group(&stream, "rogue", "testjob.>").await;

    let err = second.expect_err(
        "a work queue must reject a second overlapping consumer group - \
         without this, forgetting the shared name silently doubles every job",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("not unique") || msg.contains("workqueue") || msg.contains("filtered"),
        "expected a work-queue exclusivity error, got: {msg}"
    );
}
