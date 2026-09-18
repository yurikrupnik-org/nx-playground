//! Wire contract for project lifecycle events.
//!
//! `ProjectDeleted` crosses a **process** boundary: `zerg_api` (which owns the
//! `projects` table) publishes it, and `zerg_tasks` (which owns a separate
//! `tasks` database) consumes it to drop its now-dangling `project_id`
//! references. Two independently deployed processes agreeing on a JSON payload
//! is exactly the serialization boundary that justifies a contract crate —
//! see the rule of thumb in `docs/architecture-backlog.md` 5.1.
//!
//! It deliberately lives **here** rather than in `domain_projects`: the tasks
//! service depending on another vertical's domain crate is the shared-kernel
//! coupling Phase 2 removed, and `just boundaries` now rejects it outright
//! (`scope:tasks` may not depend on `scope:zerg`).
//!
//! # Stream shape
//!
//! The stream identity below is the half both sides must agree on. The
//! **consumer group name is not here on purpose** — it belongs to whichever
//! service is doing the consuming, which declares it via
//! `WorkerConfig::with_consumer_name`. `PROJECTS` is an [`StreamKind::EventLog`]
//! because these are domain facts, so a second group (search indexing, audit)
//! can be added later and still replay what was published before it existed.

use chrono::{DateTime, Utc};
use messaging::nats::StreamKind;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// JetStream stream carrying `projects.>` facts.
pub const PROJECTS_STREAM: &str = "PROJECTS";

/// Subject space owned by the stream. Concrete events use `projects.<fact>`.
pub const PROJECTS_SUBJECT: &str = "projects.>";

/// Dead-letter stream for `PROJECTS`.
pub const PROJECTS_DLQ: &str = "PROJECTS_DLQ";

/// Domain facts, not jobs: retained by age/count so a group added tomorrow can
/// still read what was published today.
pub const PROJECTS_KIND: StreamKind = StreamKind::EventLog;

/// A project was deleted and its id will never resolve again.
///
/// Carries no project snapshot: the only thing a consumer can act on is "this
/// id is gone", and shipping a copy of a deleted row invites readers to treat
/// the event as a substitute for the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDeleted {
    pub event_id: Uuid,
    pub project_id: Uuid,
    pub occurred_at: DateTime<Utc>,
}

impl ProjectDeleted {
    /// Concrete subject this event publishes to, within [`PROJECTS_SUBJECT`].
    pub const SUBJECT: &'static str = "projects.deleted";

    pub fn new(project_id: Uuid) -> Self {
        Self {
            event_id: Uuid::now_v7(),
            project_id,
            occurred_at: Utc::now(),
        }
    }
}

impl messaging::Job for ProjectDeleted {
    fn job_id(&self) -> Uuid {
        self.event_id
    }

    fn job_type(&self) -> &'static str {
        "project_deleted"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The concrete subject must fall inside the stream's subject space, or the
    /// publish succeeds against no stream and the event is silently dropped.
    #[test]
    fn subject_is_within_the_stream_subject_space() {
        let prefix = PROJECTS_SUBJECT.trim_end_matches('>');
        assert!(
            ProjectDeleted::SUBJECT.starts_with(prefix),
            "{} is outside {PROJECTS_SUBJECT}",
            ProjectDeleted::SUBJECT
        );
    }

    /// Field names are the wire format; a rename is a breaking change that this
    /// test makes visible at review time rather than at deploy time.
    #[test]
    fn payload_shape_is_stable() {
        let id = Uuid::now_v7();
        let event = ProjectDeleted::new(id);
        let json = serde_json::to_value(&event).expect("serialize");

        assert_eq!(json["project_id"], serde_json::json!(id));
        assert!(json.get("event_id").is_some());
        assert!(json.get("occurred_at").is_some());
        assert_eq!(
            json.as_object().expect("object").len(),
            3,
            "an added field must be reviewed: consumers deserialize this payload"
        );

        let round_tripped: ProjectDeleted = serde_json::from_value(json).expect("deserialize");
        assert_eq!(round_tripped, event);
    }
}
