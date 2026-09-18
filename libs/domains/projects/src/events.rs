//! Project lifecycle events.
//!
//! The payload and stream identity live in `contract_projects`, not here: the
//! consumer is a *different process* (`zerg_tasks`), and it must not depend on
//! this crate to name the event — see that crate's docs.
//!
//! Publishing is abstracted behind a trait so the service stays
//! transport-agnostic and unit-testable without a live NATS connection, the
//! same shape `domain_todo` uses.

use async_trait::async_trait;
use contract_projects::ProjectDeleted;

use crate::error::ProjectResult;

/// Publishes project lifecycle facts.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ProjectEventPublisher: Send + Sync {
    async fn publish_deleted(&self, event: ProjectDeleted) -> ProjectResult<()>;
}

/// No-op publisher for tests and for binaries that serve projects without
/// running the event stream.
#[derive(Clone, Default)]
pub struct NoopProjectPublisher;

#[async_trait]
impl ProjectEventPublisher for NoopProjectPublisher {
    async fn publish_deleted(&self, _event: ProjectDeleted) -> ProjectResult<()> {
        Ok(())
    }
}
