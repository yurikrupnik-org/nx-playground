//! # Tasks Contract
//!
//! The published contract for the tasks service: the DTOs that cross the wire and
//! the proto ↔ DTO conversions, and nothing else.
//!
//! Both sides of the boundary depend on this crate and on nothing of each other's:
//!
//! ```text
//!            contract_tasks
//!             ↑          ↑
//!        zerg_api    domain_tasks
//!        (client)    (server-only: entity, repository, service, postgres)
//! ```
//!
//! Storage, business logic, and HTTP routing deliberately live outside. If you find
//! yourself adding a repository, a `DatabaseConnection`, or an axum handler here, the
//! type belongs in `domain_tasks` or in the calling application instead - see
//! `docs/adr-tasks-service-boundary.md`.
//!
//! The `orm` feature is server-only; it attaches SeaORM `ActiveEnum` derives to the
//! task enums so `domain_tasks` can map them to Postgres enum columns.

pub mod conversions;
pub mod models;

pub use conversions::ConversionError;
pub use models::{CreateTask, Task, TaskFilter, TaskPriority, TaskScope, TaskStatus, UpdateTask};
