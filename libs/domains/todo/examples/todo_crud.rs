//! Simple CRUD walkthrough for the todo domain.
//!
//! Drives the full lifecycle — create, read, list, update, complete, count,
//! delete — through [`TodoService`] on top of a real Postgres, and leaves the
//! table as it found it. This is the counterpart to the event-based example in
//! `apps/zerg/email-nats/examples/`.
//!
//! Run against a local dev database:
//!
//! ```text
//! just db-fresh todo
//! DATABASE_URL=postgres://myuser:mypassword@localhost:5432/todo \
//!   cargo run -p domain_todo --example todo_crud
//! ```
//!
//! `tests/examples_it.rs` runs this exact command against a throwaway Postgres
//! container, so the walkthrough cannot rot without a test going red.
//!
//! Events go to [`NoopTodoPublisher`]: this example is about persistence, and a
//! real publisher would make it require a NATS server too. The NATS path is what
//! `tests/events_it.rs` covers.

use domain_todo::models::{CreateTodo, TodoFilter, TodoPriority, UpdateTodo};
use domain_todo::{NoopTodoPublisher, PgTodoRepository, TodoService};
use std::sync::Arc;

/// Lines prefixed `step:` are asserted on by `tests/examples_it.rs` — keep them
/// and the test in sync.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://myuser:mypassword@localhost:5432/todo".to_string());

    println!("Connecting to Postgres...");
    let connection = database::postgres::connect(&database_url).await?;
    let service = TodoService::new(
        PgTodoRepository::new(connection),
        Arc::new(NoopTodoPublisher),
    );

    // ---- create -----------------------------------------------------------
    let created = service
        .create_todo(CreateTodo {
            title: "buy milk".to_string(),
            description: "2% organic".to_string(),
            priority: TodoPriority::High,
        })
        .await?;
    println!("step:created id={} title={}", created.id, created.title);

    // ---- read -------------------------------------------------------------
    let fetched = service.get_todo(created.id).await?;
    println!(
        "step:read id={} completed={} priority={:?}",
        fetched.id, fetched.completed, fetched.priority
    );

    // ---- update -----------------------------------------------------------
    let updated = service
        .update_todo(
            created.id,
            UpdateTodo {
                title: Some("buy oat milk".to_string()),
                ..Default::default()
            },
        )
        .await?;
    println!("step:updated title={}", updated.title);

    // ---- complete ---------------------------------------------------------
    // A distinct transition rather than `update(completed: true)`: the service
    // emits a different event for it.
    let completed = service.complete_todo(created.id).await?;
    println!("step:completed completed={}", completed.completed);

    // ---- list -------------------------------------------------------------
    let done = service
        .list_todos(TodoFilter {
            completed: Some(true),
            ..Default::default()
        })
        .await?;
    println!("step:listed completed_count={}", done.len());

    println!("step:counted total={}", service.count_todos().await?);

    // ---- delete -----------------------------------------------------------
    service.delete_todo(created.id).await?;
    println!("step:deleted id={}", created.id);

    // Deleting is not enough to claim it worked — read back and prove absence.
    match service.get_todo(created.id).await {
        Err(_) => println!("step:verified_absent id={}", created.id),
        Ok(_) => return Err(format!("todo {} still readable after delete", created.id).into()),
    }

    println!("CRUD walkthrough complete");
    Ok(())
}
