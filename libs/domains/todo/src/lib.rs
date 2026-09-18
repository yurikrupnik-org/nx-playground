//! # Todo Domain
//!
//! A standalone domain-driven vertical for managing todos. Layering mirrors the
//! repo convention:
//!
//! ```text
//! Service  ← validation + business logic + event emission
//!   │
//! Repository (trait + Postgres impl)  ← persistence
//!   │
//! Models / Entity  ← domain types + SeaORM bridge
//! ```
//!
//! Lifecycle changes publish [`TodoEvent`]s to NATS JetStream (`todos.>`) via a
//! [`TodoEventPublisher`]; [`NatsTodoPublisher`] is the production impl, and the
//! consumer lives in the `todo-worker` binary.
//!
//! Browser-facing realtime is sourced from the **database**, not the service: see
//! [`db_events`], which turns Postgres `NOTIFY` into [`TodoEvent`]s so any writer
//! (other replicas, the worker, the CLI, plain SQL) reaches connected UIs.

pub mod cache;
pub mod db_events;
pub mod entity;
pub mod error;
pub mod events;
pub mod handlers;
pub mod models;
pub mod nats;
pub mod postgres;
pub mod repository;
pub mod service;

pub use cache::{CachedTodoRepository, open_cache_bucket};
pub use db_events::{TodoNotification, listen as listen_for_db_changes};
pub use error::{TodoError, TodoResult};
pub use events::{NoopTodoPublisher, TodoEvent, TodoEventKind, TodoEventPublisher};
pub use handlers::{TodoApiDoc, router};
pub use models::{CreateTodo, Todo, TodoFilter, TodoPriority, UpdateTodo};
pub use nats::{NatsTodoPublisher, TodoNatsStream};
pub use postgres::PgTodoRepository;
pub use repository::TodoRepository;
pub use service::TodoService;
