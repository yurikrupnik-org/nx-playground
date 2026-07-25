//! Rust <-> Postgres integration test for the Todo repository.
//!
//! Boots a real Postgres (testcontainers) with the todo migrations applied and
//! exercises the full CRUD surface of `PgTodoRepository`.
//!
//! Requires Docker. Run: `cargo test -p domain_todo --test repository_it`.

use domain_todo::models::{CreateTodo, TodoFilter, TodoPriority, UpdateTodo};
use domain_todo::{PgTodoRepository, TodoRepository};
use test_utils::TestDatabase;

#[tokio::test]
async fn crud_roundtrip_against_postgres() {
    let db = TestDatabase::with_migrations_dir("manifests/db/todo/migrations").await;
    let repo = PgTodoRepository::new(db.connection());

    // create
    let created = repo
        .create(CreateTodo {
            title: "buy milk".into(),
            description: "2% organic".into(),
            priority: TodoPriority::High,
        })
        .await
        .expect("create");
    assert_eq!(created.title, "buy milk");
    assert_eq!(created.description, "2% organic");
    assert!(!created.completed);
    assert_eq!(created.priority, TodoPriority::High);

    // get
    let fetched = repo
        .get_by_id(created.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(fetched.id, created.id);

    // update -> complete
    let updated = repo
        .update(
            created.id,
            UpdateTodo {
                completed: Some(true),
                title: Some("buy oat milk".into()),
                ..Default::default()
            },
        )
        .await
        .expect("update");
    assert!(updated.completed);
    assert_eq!(updated.title, "buy oat milk");

    // list filters (limit must be explicit; Default gives 0 == LIMIT 0)
    let completed = repo
        .list(TodoFilter {
            completed: Some(true),
            limit: 50,
            ..Default::default()
        })
        .await
        .expect("list completed");
    assert_eq!(completed.len(), 1);

    let active = repo
        .list(TodoFilter {
            completed: Some(false),
            limit: 50,
            ..Default::default()
        })
        .await
        .expect("list active");
    assert!(active.is_empty());

    // count
    assert_eq!(repo.count().await.expect("count"), 1);

    // delete (idempotency: second delete returns false)
    assert!(repo.delete(created.id).await.expect("delete"));
    assert!(!repo.delete(created.id).await.expect("delete again"));
    assert!(repo.get_by_id(created.id).await.expect("get").is_none());
}
