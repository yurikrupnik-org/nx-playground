//! The `insights-warehouse` consumer against a real JetStream + Postgres:
//! drains to the end, applies each fact once, records poison and moves on.

#![allow(clippy::unwrap_used)]

mod common;

use chrono::Utc;
use common::{count, insights_db};
use contract_taskgraph::{EventBody, RunOutcome, TaskOutcome, TaskgraphEvent, Via};
use domain_insights::warehouse::{DrainReport, drain};
use domain_taskgraph::{EventPublisher, NatsPublisher};
use test_utils::TestNats;
use uuid::Uuid;

fn run_events(run_id: Uuid) -> Vec<TaskgraphEvent> {
    let now = Utc::now();
    vec![
        TaskgraphEvent::new(
            EventBody::TaskStarted {
                run_id,
                instance: 1,
                task: "lint".into(),
                parent: None,
                via: Via::Root,
            },
            now,
            None,
        ),
        TaskgraphEvent::new(
            EventBody::TaskFinished {
                run_id,
                instance: 1,
                task: "lint".into(),
                outcome: TaskOutcome::Failed,
                duration_ms: 20,
                error: Some("exit status 1".into()),
            },
            now,
            None,
        ),
        TaskgraphEvent::new(
            EventBody::RunFinished {
                run_id,
                outcome: RunOutcome::Failed,
                exit_code: Some(1),
                duration_ms: 25,
                error: None,
            },
            now,
            None,
        ),
    ]
}

#[tokio::test]
async fn drain_applies_each_fact_once_and_skips_poison() {
    let nats = TestNats::new().await;
    let (_db, store) = insights_db().await;
    let jetstream = nats.jetstream();
    let publisher = NatsPublisher::new(jetstream.clone()).await.unwrap();

    // Nothing published yet: the consumer is created and returns at once.
    assert_eq!(
        drain(&jetstream, &store).await.unwrap(),
        DrainReport::default()
    );

    let first = run_events(Uuid::now_v7());
    for event in &first {
        publisher.publish(event).await.unwrap();
    }
    jetstream
        .publish("taskgraph.run_started", "{not json".into())
        .await
        .unwrap()
        .await
        .unwrap();
    // More than one fetch batch, so the drain must loop until nothing is pending.
    let bulk: Vec<TaskgraphEvent> = (0..300).flat_map(|_| run_events(Uuid::now_v7())).collect();
    for event in &bulk {
        publisher.publish(event).await.unwrap();
    }

    let report = drain(&jetstream, &store).await.unwrap();
    assert_eq!(
        report,
        DrainReport {
            applied: 903,
            duplicates: 0,
            undecodable: 1
        }
    );
    assert_eq!(count(&store, "task_runs").await, 301);
    assert_eq!(count(&store, "task_executions").await, 301);
    assert_eq!(
        count(&store, "sync_errors WHERE source = 'warehouse'").await,
        1
    );

    // The same facts again, past any publish-side dedupe (a re-ingested
    // artifact after the duplicate window): the event_id ledger absorbs them.
    for event in &first {
        jetstream
            .publish(
                event.body.subject(),
                serde_json::to_vec(event).unwrap().into(),
            )
            .await
            .unwrap()
            .await
            .unwrap();
    }
    let report = drain(&jetstream, &store).await.unwrap();
    assert_eq!(
        report,
        DrainReport {
            applied: 0,
            duplicates: 3,
            undecodable: 0
        }
    );

    // The durable cursor moved: nothing is redelivered.
    assert_eq!(
        drain(&jetstream, &store).await.unwrap(),
        DrainReport::default()
    );
    assert_eq!(count(&store, "tg_events").await, 903);
}
