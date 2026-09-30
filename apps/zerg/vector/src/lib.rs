//! Vector gRPC service.
//!
//! Split out of `zerg_tasks` so the two can scale and deploy independently - while they
//! shared a binary, "scale tasks independently" was false by construction. See
//! `docs/adr-tasks-service-boundary.md` Phase 5.

pub mod server;
pub mod vector_service;

pub use server::run;
pub use vector_service::VectorServiceImpl;
