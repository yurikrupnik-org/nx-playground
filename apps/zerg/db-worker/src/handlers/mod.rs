//! Event handlers per database backend.

pub mod dapr_state;

#[cfg(feature = "postgres")]
pub mod tasks_pg;

#[cfg(feature = "mongo")]
pub mod tasks_mongo;

#[cfg(feature = "qdrant")]
pub mod qdrant;

#[cfg(feature = "influxdb")]
pub mod influxdb;

#[cfg(feature = "neo4j")]
pub mod neo4j;
