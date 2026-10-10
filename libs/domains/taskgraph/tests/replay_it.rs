//! Publish through the real producer into a real JetStream, rebuild through
//! the real ordered-consumer replay: the path both the API (on start) and the
//! CLI (`runs`, `estimate`) depend on. Requires docker (testcontainers).

use std::time::Duration;

use chrono::Utc;
use contract_taskgraph::{EventBody, RunOrigin, RunOutcome, TaskOutcome, TaskgraphEvent, Via};
use domain_taskgraph::nats::replay;
use domain_taskgraph::{EventPublisher, Limits, NatsPublisher, Projection};
use test_utils::TestNats;
use uuid::Uuid;

const DEADLINE: Duration = Duration::from_secs(20);

#[tokio::test]
async fn replay_rebuilds_runs_from_the_stream_and_stops_when_caught_up() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();

    // An empty stream must not block waiting for a first message.
    let mut empty = Projection::new(Limits::default());
    let read = tokio::time::timeout(DEADLINE, replay(&js, &mut empty))
        .await
        .expect("replay of an empty stream returns")
        .expect("replay");
    assert_eq!(read, 0);

    let publisher = NatsPublisher::new(js.clone()).await.expect("publisher");
    let run_id = Uuid::now_v7();
    let bodies = [
        EventBody::RunStarted {
            run_id,
            graph_id: "g".into(),
            target: "build".into(),
            args: vec![],
            host: "h".into(),
            user: "u".into(),
            cwd: "/".into(),
            estimate_ms: None,
            origin: RunOrigin::default(),
        },
        EventBody::TaskStarted {
            run_id,
            instance: 1,
            task: "build".into(),
            parent: None,
            via: Via::Root,
        },
        EventBody::CommandStarted {
            run_id,
            instance: 1,
            task: "build".into(),
            command: "cargo build".into(),
        },
        EventBody::TaskFinished {
            run_id,
            instance: 1,
            task: "build".into(),
            outcome: TaskOutcome::Succeeded,
            duration_ms: 1_500,
            error: None,
        },
        EventBody::RunFinished {
            run_id,
            outcome: RunOutcome::Succeeded,
            exit_code: Some(0),
            duration_ms: 1_600,
            error: None,
        },
    ];
    for body in bodies {
        publisher
            .publish(&TaskgraphEvent::new(body, Utc::now(), None))
            .await
            .expect("publish");
    }

    let mut projection = Projection::new(Limits::default());
    let read = tokio::time::timeout(DEADLINE, replay(&js, &mut projection))
        .await
        .expect("replay stops once caught up")
        .expect("replay");
    assert_eq!(read, 5);

    let run = projection.run(run_id).expect("run rebuilt");
    assert_eq!(run.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(run.executions[0].commands[0].command, "cargo build");
    assert_eq!(projection.stats("g", "build").p50_ms, Some(1_500));
}

#[tokio::test]
async fn republishing_an_event_is_deduplicated_by_its_id() {
    let nats = TestNats::new().await;
    let js = nats.jetstream();
    let publisher = NatsPublisher::new(js.clone()).await.expect("publisher");
    let event = TaskgraphEvent::new(
        EventBody::RunFinished {
            run_id: Uuid::now_v7(),
            outcome: RunOutcome::Failed,
            exit_code: Some(201),
            duration_ms: 10,
            error: None,
        },
        Utc::now(),
        None,
    );
    assert!(!publisher.send(&event).await.expect("first publish"));
    // Same event again (an artifact ingested twice): acked, not stored.
    assert!(publisher.send(&event).await.expect("second publish"));
    // A distinct event with the same body is a new fact.
    let other = TaskgraphEvent::new(event.body.clone(), event.at, None);
    assert!(!publisher.send(&other).await.expect("third publish"));

    let mut projection = Projection::new(Limits::default());
    let read = tokio::time::timeout(DEADLINE, replay(&js, &mut projection))
        .await
        .expect("replay returns")
        .expect("replay");
    assert_eq!(read, 2);
}
