//! Tasks Domain — server-side implementation behind the `contract_tasks` wire contract.
//!
//! This crate holds the service, the repository, and the SeaORM entity. It does **not**
//! define the DTOs and it no longer carries HTTP/gRPC handlers: transport lives in
//! `apps/zerg/tasks`, which is the only consumer. Callers depend on `contract_tasks`
//! alone (see `docs/adr-tasks-service-boundary.md`, checklist row 1).
//!
//! # Architecture
//!
//! ```text
//!  contract_tasks  (published wire contract — separate crate)
//!  ┌───────────────────────────────────────────────┐
//!  │  models: Task, CreateTask, UpdateTask,        │
//!  │          TaskFilter, TaskScope, enums         │
//!  │  conversions: proto <-> domain                │
//!  └───────────────────────┬───────────────────────┘
//!            re-exported    │    (never extended here —
//!            for callers    │     adding a field changes the wire)
//!  ═══════════════════════ crate boundary ═════════════════════════
//!  domain_tasks (this crate)
//!  ┌───────────────────────▼───────────────────────┐
//!  │  Service     │  validation, tenant scoping    │
//!  ├──────────────┼────────────────────────────────┤
//!  │  Repository  │  trait; PgTaskRepository impl   │
//!  ├──────────────┼────────────────────────────────┤
//!  │  entity      │  SeaORM row type — local only,  │
//!  │              │  never crosses the boundary     │
//!  └──────────────┴────────────────────────────────┘
//! ```
//!
//! # Tenancy
//!
//! Every service method takes a [`TaskScope`] (or a bare `org_ref`) and the repository
//! filters on it. This is the isolation invariant: there is no unscoped read or write
//! path, so a caller cannot reach another organization's rows by omitting a filter.
//!
//! # Usage
//!
//! ```rust,no_run
//! use domain_tasks::{
//!     CreateTask, PgTaskRepository, TaskPriority, TaskScope, TaskService, TaskStatus,
//! };
//! use sea_orm::Database;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let db = Database::connect("postgres://...").await?;
//! let service = TaskService::new(PgTaskRepository::new(db));
//!
//! // The scope is not optional — it is how the row is owned and later found.
//! let scope = TaskScope {
//!     org_ref: "org_123".to_string(),
//!     user_ref: "user_456".to_string(),
//! };
//!
//! let task = service
//!     .create_task(scope.clone(), CreateTask {
//!         title: "ship the boundary fix".to_string(),
//!         description: String::new(),
//!         project_id: None,
//!         priority: TaskPriority::High,
//!         status: TaskStatus::Todo,
//!         due_date: None,
//!     })
//!     .await?;
//!
//! // Reads are scoped by the same org, so another tenant's id yields `NotFound`.
//! let fetched = service.get_task(&scope.org_ref, task.id).await?;
//! assert_eq!(fetched.id, task.id);
//! # Ok(())
//! # }
//! ```

pub mod entity;
pub mod error;
pub mod postgres;
pub mod repository;
pub mod service;

// The published contract, re-exported so this crate's internals can keep referring to
// `crate::models` / `crate::conversions` and so server-side consumers need only one
// dependency line. The DTOs are *defined* in `contract_tasks`; nothing here may add to
// them without changing the wire contract.
pub use contract_tasks::{conversions, models};

// Re-export commonly used types
pub use contract_tasks::{
    CreateTask, Task, TaskFilter, TaskPriority, TaskScope, TaskStatus, UpdateTask,
};
pub use error::{TaskError, TaskResult};
pub use postgres::PgTaskRepository;
pub use repository::TaskRepository;
pub use service::TaskService;

// Re-export ApiResource trait for accessing generated constants
pub use core_proc_macros::ApiResource;
