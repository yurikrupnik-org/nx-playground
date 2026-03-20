//! Database operation event types.
//!
//! These events flow through Dapr pub/sub (backed by NATS JetStream)
//! and are processed by db-worker instances.

use crate::{Job, JobPriority};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A database operation event delivered via Dapr pub/sub.
///
/// Each event targets a specific database backend and entity type.
/// The worker deployment for the matching backend picks it up and
/// executes the operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbOpEvent {
    /// Unique event ID.
    pub id: String,
    /// The database operation to perform.
    pub operation: DbOperation,
    /// The entity type (e.g., "tasks", "users", "projects").
    pub entity_type: String,
    /// The operation payload (entity data, query parameters, etc.).
    pub payload: serde_json::Value,
    /// Current retry count.
    #[serde(default)]
    pub retry_count: u32,
    /// Optional correlation ID for tracing across services.
    #[serde(default)]
    pub correlation_id: Option<String>,
}

impl DbOpEvent {
    /// Create a new DB operation event.
    pub fn new(
        operation: DbOperation,
        entity_type: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            operation,
            entity_type: entity_type.into(),
            payload,
            retry_count: 0,
            correlation_id: None,
        }
    }

    /// Set a correlation ID for distributed tracing.
    pub fn with_correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }
}

impl Job for DbOpEvent {
    fn job_id(&self) -> String {
        self.id.clone()
    }

    fn retry_count(&self) -> u32 {
        self.retry_count
    }

    fn with_retry(&self) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            retry_count: self.retry_count + 1,
            ..self.clone()
        }
    }

    fn max_retries(&self) -> u32 {
        5
    }

    fn priority(&self) -> JobPriority {
        match self.operation {
            DbOperation::Create | DbOperation::Update | DbOperation::Delete => JobPriority::High,
            DbOperation::Read | DbOperation::Query => JobPriority::Normal,
            _ => JobPriority::Normal,
        }
    }

    fn job_type(&self) -> &'static str {
        "db_op_event"
    }
}

/// Database operations supported by the worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbOperation {
    // Standard CRUD
    Create,
    Read,
    Update,
    Delete,
    Query,

    // Vector-specific (Qdrant)
    VectorUpsert,
    VectorSearch,

    // Time-series-specific (InfluxDB)
    TimeSeriesWrite,
    TimeSeriesQuery,

    // Graph-specific (Neo4j)
    GraphTraverse,
    GraphMutate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_op_event_creation() {
        let event = DbOpEvent::new(
            DbOperation::Create,
            "tasks",
            serde_json::json!({"title": "Test task"}),
        );
        assert_eq!(event.entity_type, "tasks");
        assert_eq!(event.operation, DbOperation::Create);
        assert_eq!(event.retry_count, 0);
        assert!(event.can_retry());
    }

    #[test]
    fn test_db_op_event_retry() {
        let event = DbOpEvent::new(
            DbOperation::Create,
            "tasks",
            serde_json::json!({"title": "Test"}),
        );
        let retried = event.with_retry();
        assert_eq!(retried.retry_count, 1);
        assert_ne!(retried.id, event.id);
    }

    #[test]
    fn test_db_op_event_serialization() {
        let event = DbOpEvent::new(
            DbOperation::VectorSearch,
            "embeddings",
            serde_json::json!({"query": [0.1, 0.2, 0.3]}),
        );
        let json = serde_json::to_string(&event).unwrap();
        let parsed: DbOpEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.operation, DbOperation::VectorSearch);
        assert_eq!(parsed.entity_type, "embeddings");
    }

    #[test]
    fn test_priority_for_operations() {
        let create = DbOpEvent::new(DbOperation::Create, "tasks", serde_json::Value::Null);
        assert_eq!(create.priority(), JobPriority::High);

        let read = DbOpEvent::new(DbOperation::Read, "tasks", serde_json::Value::Null);
        assert_eq!(read.priority(), JobPriority::Normal);
    }
}
