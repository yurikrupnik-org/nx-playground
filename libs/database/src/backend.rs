//! Database backend abstraction for multi-database architectures.
//!
//! Defines the supported database backends and a unified client enum
//! for accessing them via Dapr state stores or direct clients.

use serde::{Deserialize, Serialize};

/// Supported database backends.
///
/// Each variant maps to a specific database technology. The `strum` derives
/// allow parsing from environment variables (e.g., `DB_BACKEND=postgres`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, strum::EnumString, strum::Display)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum DatabaseBackend {
    Postgres,
    Mongo,
    Qdrant,
    InfluxDb,
    Neo4j,
}

impl DatabaseBackend {
    /// Returns the Dapr state store component name for backends that use Dapr.
    ///
    /// Returns `None` for backends that require direct clients (Qdrant, InfluxDB, Neo4j).
    pub fn dapr_state_store_name(&self) -> Option<&'static str> {
        match self {
            DatabaseBackend::Postgres => Some("statestore-postgres"),
            DatabaseBackend::Mongo => Some("statestore-mongo"),
            _ => None,
        }
    }

    /// Returns the NATS topic this backend listens on.
    pub fn topic(&self) -> &'static str {
        match self {
            DatabaseBackend::Postgres => "db.tasks.pg",
            DatabaseBackend::Mongo => "db.tasks.mongo",
            DatabaseBackend::Qdrant => "db.vectors",
            DatabaseBackend::InfluxDb => "db.timeseries",
            DatabaseBackend::Neo4j => "db.graph",
        }
    }

    /// Whether this backend is managed via Dapr state store API.
    pub fn uses_dapr_state_store(&self) -> bool {
        self.dapr_state_store_name().is_some()
    }
}

/// Unified database client supporting both Dapr-managed and direct connections.
pub enum DatabaseClient {
    /// PostgreSQL accessed via Dapr state store API.
    DaprPostgres { state_store_name: String },
    /// MongoDB accessed via Dapr state store API.
    DaprMongo { state_store_name: String },
    /// Qdrant vector database (direct client - Dapr doesn't support vector DBs).
    #[cfg(feature = "qdrant")]
    Qdrant(qdrant_client::Qdrant),
    /// InfluxDB time-series database (direct client).
    #[cfg(feature = "influxdb")]
    InfluxDb(influxdb2::Client),
    /// Neo4j graph database (direct client).
    #[cfg(feature = "neo4j")]
    Neo4j(neo4rs::Graph),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_parse_backend_from_str() {
        assert_eq!(DatabaseBackend::from_str("postgres").unwrap(), DatabaseBackend::Postgres);
        assert_eq!(DatabaseBackend::from_str("mongo").unwrap(), DatabaseBackend::Mongo);
        assert_eq!(DatabaseBackend::from_str("qdrant").unwrap(), DatabaseBackend::Qdrant);
        assert_eq!(DatabaseBackend::from_str("influxdb").unwrap(), DatabaseBackend::InfluxDb);
        assert_eq!(DatabaseBackend::from_str("neo4j").unwrap(), DatabaseBackend::Neo4j);
    }

    #[test]
    fn test_dapr_state_store_name() {
        assert_eq!(DatabaseBackend::Postgres.dapr_state_store_name(), Some("statestore-postgres"));
        assert_eq!(DatabaseBackend::Mongo.dapr_state_store_name(), Some("statestore-mongo"));
        assert_eq!(DatabaseBackend::Qdrant.dapr_state_store_name(), None);
        assert_eq!(DatabaseBackend::InfluxDb.dapr_state_store_name(), None);
        assert_eq!(DatabaseBackend::Neo4j.dapr_state_store_name(), None);
    }

    #[test]
    fn test_topic() {
        assert_eq!(DatabaseBackend::Postgres.topic(), "db.tasks.pg");
        assert_eq!(DatabaseBackend::Qdrant.topic(), "db.vectors");
    }

    #[test]
    fn test_serialization() {
        let backend = DatabaseBackend::Postgres;
        let json = serde_json::to_string(&backend).unwrap();
        assert_eq!(json, "\"postgres\"");
        let parsed: DatabaseBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, DatabaseBackend::Postgres);
    }
}
