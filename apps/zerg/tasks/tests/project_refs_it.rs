//! End-to-end proof for backlog 0.3: a deleted project stops dangling in the
//! tasks service.
//!
//! Publishes a `ProjectDeleted` into a real JetStream, runs the real
//! `NatsWorker` with the real `ProjectRefsProcessor` against a real Postgres
//! holding the tasks schema, and asserts the task's `project_id` becomes NULL
//! while the task itself still loads.
//!
//! The publish deliberately uses **only `contract_projects` + `messaging`**,
//! never `domain_projects`: `zerg_tasks` must not depend on another vertical's
//! domain crate (`just boundaries` rejects it), and a test dependency is a real
//! cargo edge. Publishing straight to the contract's subject is also the
//! stronger assertion — it proves the two sides agree on stream, subject and
//! payload rather than sharing one helper that could be wrong in the same way
//! on both sides.
//!
//! Requires Docker. Run: `cargo test -p zerg_tasks --test project_refs_it`.

use std::time::{Duration, Instant};

use contract_projects::{PROJECTS_KIND, PROJECTS_STREAM, PROJECTS_SUBJECT, ProjectDeleted};
use contract_tasks::{CreateTask, TaskPriority, TaskScope, TaskStatus};
use domain_tasks::{PgTaskRepository, TaskService};
use messaging::nats::{NatsProducer, NatsWorker, WorkerConfig, stream_config_for};
use test_utils::{TestDatabase, TestNats};
use uuid::Uuid;
use zerg_tasks::{ProjectRefsProcessor, ProjectRefsStream};

fn scope() -> TaskScope {
    TaskScope {
        org_ref: "org_01TESTPROJECTREFS".to_string(),
        user_ref: "user_01TESTPROJECTREFS".to_string(),
    }
}

#[tokio::test]
async fn a_deleted_project_clears_its_task_references() {
    let db = TestDatabase::with_migrations_dir("manifests/db/tasks/schema.sql").await;
    let tasks = TaskService::new(PgTaskRepository::new(db.connection()));

    // Two tasks in the doomed project, one in a project that survives: the
    // clear must be exact, not a blanket wipe of every project reference.
    let doomed = Uuid::now_v7();
    let survivor = Uuid::now_v7();
    let mut in_doomed = Vec::new();
    for title in ["first", "second"] {
        let task = tasks
            .create_task(scope(), new_task(title, Some(doomed)))
            .await
            .expect("create task in the doomed project");
        in_doomed.push(task.id);
    }
    let untouched = tasks
        .create_task(scope(), new_task("other project", Some(survivor)))
        .await
        .expect("create task in the surviving project");

    // Publish the fact exactly as `zerg_api` would: the contract's stream, the
    // contract's subject, the contract's payload.
    let nats = TestNats::new().await;
    let js = nats.jetstream();
    js.get_or_create_stream(stream_config_for(
        PROJECTS_STREAM,
        PROJECTS_SUBJECT,
        PROJECTS_KIND,
    ))
    .await
    .expect("create PROJECTS stream");
    NatsProducer::new(js.clone(), PROJECTS_STREAM, PROJECTS_SUBJECT)
        .send_to(ProjectDeleted::SUBJECT, &ProjectDeleted::new(doomed))
        .await
        .expect("publish ProjectDeleted");

    let worker = NatsWorker::<ProjectDeleted, _>::new(
        js.clone(),
        ProjectRefsProcessor::new(tasks.clone()),
        WorkerConfig::from_stream::<ProjectRefsStream>(),
    )
    .await
    .expect("create the project-refs worker");

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(async move { worker.run(shutdown_rx).await });

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let task = tasks
            .get_task(&scope().org_ref, in_doomed[0])
            .await
            .expect("the task must still load after its project is deleted");
        if task.project_id.is_none() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "project reference was not cleared within the timeout"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Every task in the deleted project is cleared, and each still loads — the
    // acceptance criterion is "no error, no phantom project".
    for id in &in_doomed {
        let task = tasks
            .get_task(&scope().org_ref, *id)
            .await
            .expect("task loads");
        assert_eq!(
            task.project_id, None,
            "task {id} still references the deleted project"
        );
    }

    let survivor_task = tasks
        .get_task(&scope().org_ref, untouched.id)
        .await
        .expect("task in the surviving project loads");
    assert_eq!(
        survivor_task.project_id,
        Some(survivor),
        "a different project's tasks must be untouched"
    );

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}

/// Redelivery is expected under at-least-once, and the correction is defined so
/// that repeating it is a no-op — this is why the consumer needs no dedupe
/// (contrast backlog 0.2, where the side effect is an email).
#[tokio::test]
async fn clearing_the_same_project_twice_is_a_no_op() {
    let db = TestDatabase::with_migrations_dir("manifests/db/tasks/schema.sql").await;
    let tasks = TaskService::new(PgTaskRepository::new(db.connection()));

    let project_id = Uuid::now_v7();
    tasks
        .create_task(scope(), new_task("only", Some(project_id)))
        .await
        .expect("create task");

    let first = tasks
        .clear_project_refs(project_id)
        .await
        .expect("first clear");
    let second = tasks
        .clear_project_refs(project_id)
        .await
        .expect("second clear");

    assert_eq!(first, 1, "the one referencing task is cleared");
    assert_eq!(second, 0, "a replay changes nothing");
}

fn new_task(title: &str, project_id: Option<Uuid>) -> CreateTask {
    CreateTask {
        title: title.to_string(),
        description: String::new(),
        project_id,
        priority: TaskPriority::Medium,
        status: TaskStatus::Todo,
        due_date: None,
    }
}
