//! Consumes `ProjectDeleted` and drops this service's references to the id.
//!
//! ## Why a consumer at all
//!
//! Phase 3 gave tasks its own database, so `tasks.project_id` cannot have a
//! foreign key into `projects` — that table lives in another database owned by
//! another service. `ON DELETE SET NULL` went away with the FK, and nothing
//! replaced it: deleting a project left every task in it pointing at an id that
//! resolves to nothing (`docs/architecture-backlog.md` 0.3).
//!
//! An ID reference plus an event is the modular-monolith answer to a
//! cross-context FK, and this is that event.
//!
//! ## Delivery properties this relies on
//!
//! - **Idempotent by construction.** The work is "null the refs to this id", so
//!   a redelivery clears zero rows. At-least-once needs no dedupe here, unlike
//!   the email path in backlog 0.2.
//! - **Durable catch-up.** `PROJECTS` is an `EventLog` (retained by age/count,
//!   not consumption) and this group's cursor is server-side, so a tasks
//!   service that is down when a project is deleted still applies the
//!   correction when it comes back. A NATS outage delays the fix; it does not
//!   lose it.
//! - **Its own consumer group.** `CONSUMER_NAME` is declared here rather than in
//!   `contract_projects`, because a group belongs to the service that reads it.
//!   A second reader (search, audit) adds its own name and gets its own copy.

use contract_projects::{
    PROJECTS_DLQ, PROJECTS_KIND, PROJECTS_STREAM, PROJECTS_SUBJECT, ProjectDeleted,
};
use domain_tasks::{TaskRepository, TaskService};
use messaging::nats::{StreamConfig, StreamKind};
use messaging::{ProcessingError, Processor};
use tracing::{debug, info};

/// This service's view of the shared `PROJECTS` stream.
pub struct ProjectRefsStream;

impl StreamConfig for ProjectRefsStream {
    const STREAM_NAME: &'static str = PROJECTS_STREAM;
    /// The tasks service's consumer group. Every replica shares it, so one
    /// replica applies each deletion rather than all of them redundantly.
    const CONSUMER_NAME: &'static str = "tasks-project-refs";
    const DLQ_STREAM: &'static str = PROJECTS_DLQ;
    const SUBJECT: &'static str = PROJECTS_SUBJECT;
    const KIND: StreamKind = PROJECTS_KIND;
}

/// Nulls `tasks.project_id` for a deleted project.
pub struct ProjectRefsProcessor<R: TaskRepository> {
    tasks: TaskService<R>,
}

impl<R: TaskRepository> ProjectRefsProcessor<R> {
    pub fn new(tasks: TaskService<R>) -> Self {
        Self { tasks }
    }
}

impl<R: TaskRepository> Processor<ProjectDeleted> for ProjectRefsProcessor<R> {
    #[tracing::instrument(skip_all, fields(project_id = %job.project_id, event_id = %job.event_id))]
    async fn process(&self, job: &ProjectDeleted) -> Result<(), ProcessingError> {
        // Transient, not permanent: the only realistic failure is the database
        // being unreachable, and a DLQ'd deletion would leave the refs dangling
        // forever with no operator signal that a row needs fixing.
        let cleared = self
            .tasks
            .clear_project_refs(job.project_id)
            .await
            .map_err(|e| ProcessingError::transient(e.to_string()))?;

        if cleared > 0 {
            info!(
                project_id = %job.project_id,
                tasks_cleared = cleared,
                "cleared task references to deleted project"
            );
        } else {
            // Normal: no task used the project, or a redelivery already applied
            // this. Both are successes, not anomalies.
            debug!(project_id = %job.project_id, "no task referenced the deleted project");
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "project_refs_processor"
    }
}
