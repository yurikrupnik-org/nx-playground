//! Wire-contract test (no external services).
//!
//! Pins the JSON shape that BOTH frontends depend on: the SolidJS app via the
//! ts-rs generated `@domain/todo` types, and the Rust/Leptos app via its own
//! mirrored serde structs. If a field is renamed/added here, this fails before
//! the frontends drift.

use chrono::Utc;
use domain_todo::models::{CreateTodo, Todo, TodoPriority};
use uuid::Uuid;

#[test]
fn todo_json_has_expected_fields() {
    let todo = Todo {
        id: Uuid::now_v7(),
        title: "t".into(),
        description: "d".into(),
        completed: false,
        priority: TodoPriority::Low,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let v = serde_json::to_value(&todo).expect("serialize");
    for key in [
        "id",
        "title",
        "description",
        "completed",
        "priority",
        "created_at",
        "updated_at",
    ] {
        assert!(v.get(key).is_some(), "Todo JSON missing field `{key}`");
    }
    assert!(v["id"].is_string(), "id must serialize as string");
}

#[test]
fn priority_serializes_lowercase() {
    assert_eq!(
        serde_json::to_value(TodoPriority::High).unwrap(),
        serde_json::json!("high")
    );
    assert_eq!(
        serde_json::to_value(TodoPriority::Medium).unwrap(),
        serde_json::json!("medium")
    );
}

#[test]
fn create_todo_deserializes_with_defaults() {
    // Frontends may omit description/priority; serde defaults must hold.
    let c: CreateTodo = serde_json::from_str(r#"{"title":"only title"}"#).expect("deserialize");
    assert_eq!(c.title, "only title");
    assert_eq!(c.description, "");
    assert_eq!(c.priority, TodoPriority::Medium);
}
