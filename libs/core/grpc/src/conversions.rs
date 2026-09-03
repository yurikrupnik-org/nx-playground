//! Generic protobuf ↔ domain conversions
//!
//! Provides reusable conversion functions for common types used in protobuf schemas:
//! - UUIDs (domain Uuid ↔ protobuf bytes)
//! - Timestamps (domain DateTime<Utc> ↔ protobuf i64 Unix timestamps)
//!
//! These helpers are domain-agnostic and can be used across all services
//! (tasks, users, projects, cloud_resources, etc.)
//!
//! ## Usage
//!
//! ```ignore
//! use grpc_client::conversions::*;
//! use uuid::Uuid;
//! use chrono::{DateTime, Utc};
//!
//! // UUID conversions
//! let uuid = Uuid::new_v4();
//! let bytes = uuid_to_bytes(uuid);
//! let uuid_back = bytes_to_uuid(&bytes).unwrap();
//!
//! // Timestamp conversions
//! let now = Utc::now();
//! let timestamp = datetime_to_timestamp(now);
//! let dt_back = timestamp_to_datetime(timestamp);
//! ```

use chrono::{DateTime, Utc};
use uuid::Uuid;

// ============================================================================
// UUID Conversions (protobuf bytes ↔ Uuid)
// ============================================================================

/// Convert a UUID to protobuf bytes representation
///
/// # Example
/// ```ignore
/// let uuid = Uuid::new_v4();
/// let bytes = uuid_to_bytes(uuid);
/// assert_eq!(bytes.len(), 16);
/// ```
pub fn uuid_to_bytes(uuid: Uuid) -> Vec<u8> {
    uuid.as_bytes().to_vec()
}

/// Convert protobuf bytes to UUID
///
/// Returns an error if the byte slice is not exactly 16 bytes.
///
/// # Example
/// ```ignore
/// let uuid = Uuid::new_v4();
/// let bytes = uuid.as_bytes();
/// let uuid_back = bytes_to_uuid(bytes).unwrap();
/// assert_eq!(uuid, uuid_back);
/// ```
pub fn bytes_to_uuid(bytes: &[u8]) -> Result<Uuid, String> {
    Uuid::from_slice(bytes).map_err(|e| format!("Invalid UUID bytes: {e}"))
}

/// Convert optional UUID to optional protobuf bytes
///
/// # Example
/// ```ignore
/// let uuid = Some(Uuid::new_v4());
/// let bytes = opt_uuid_to_bytes(uuid);
/// assert!(bytes.is_some());
/// ```
pub fn opt_uuid_to_bytes(uuid: Option<Uuid>) -> Option<Vec<u8>> {
    uuid.map(uuid_to_bytes)
}

/// Convert optional protobuf bytes to optional UUID
///
/// Returns an error if bytes are present but invalid.
///
/// # Example
/// ```ignore
/// let uuid = Uuid::new_v4();
/// let bytes = Some(uuid.as_bytes().to_vec());
/// let uuid_back = opt_bytes_to_uuid(bytes).unwrap();
/// assert_eq!(Some(uuid), uuid_back);
/// ```
pub fn opt_bytes_to_uuid(bytes: Option<Vec<u8>>) -> Result<Option<Uuid>, String> {
    bytes.map(|b| bytes_to_uuid(&b)).transpose()
}

// ============================================================================
// Timestamp Conversions (Unix timestamp ↔ DateTime<Utc>)
// ============================================================================

/// Convert DateTime<Utc> to Unix timestamp (seconds since epoch)
///
/// # Example
/// ```ignore
/// let now = Utc::now();
/// let timestamp = datetime_to_timestamp(now);
/// assert!(timestamp > 0);
/// ```
pub fn datetime_to_timestamp(dt: DateTime<Utc>) -> i64 {
    dt.timestamp()
}

/// Convert Unix timestamp to DateTime<Utc>
///
/// Falls back to current time if the timestamp is invalid.
///
/// # Example
/// ```ignore
/// let timestamp = 1702209600; // 2023-12-10
/// let dt = timestamp_to_datetime(timestamp);
/// assert_eq!(dt.timestamp(), timestamp);
/// ```
pub fn timestamp_to_datetime(timestamp: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(timestamp, 0).unwrap_or_else(Utc::now)
}

/// Convert optional Unix timestamp to optional DateTime<Utc>
///
/// # Example
/// ```ignore
/// let timestamp = Some(1702209600);
/// let dt = opt_timestamp_to_datetime(timestamp);
/// assert!(dt.is_some());
/// ```
pub fn opt_timestamp_to_datetime(timestamp: Option<i64>) -> Option<DateTime<Utc>> {
    timestamp.map(timestamp_to_datetime)
}

/// Convert optional DateTime<Utc> to optional Unix timestamp
///
/// # Example
/// ```ignore
/// let now = Some(Utc::now());
/// let timestamp = opt_datetime_to_timestamp(now);
/// assert!(timestamp.is_some());
/// ```
pub fn opt_datetime_to_timestamp(dt: Option<DateTime<Utc>>) -> Option<i64> {
    dt.map(datetime_to_timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uuid_roundtrip() {
        let uuid = Uuid::new_v4();
        let bytes = uuid_to_bytes(uuid);
        let uuid_back = bytes_to_uuid(&bytes).unwrap();
        assert_eq!(uuid, uuid_back);
    }

    #[test]
    fn test_opt_uuid_roundtrip() {
        let uuid = Some(Uuid::new_v4());
        let bytes = opt_uuid_to_bytes(uuid);
        let uuid_back = opt_bytes_to_uuid(bytes).unwrap();
        assert_eq!(uuid, uuid_back);
    }

    #[test]
    fn test_opt_uuid_none() {
        let bytes = opt_uuid_to_bytes(None);
        assert!(bytes.is_none());
        let uuid = opt_bytes_to_uuid(None).unwrap();
        assert!(uuid.is_none());
    }

    #[test]
    fn test_timestamp_roundtrip() {
        let now = Utc::now();
        let timestamp = datetime_to_timestamp(now);
        let dt_back = timestamp_to_datetime(timestamp);
        assert_eq!(now.timestamp(), dt_back.timestamp());
    }

    #[test]
    fn test_opt_timestamp_roundtrip() {
        let now = Utc::now();
        let timestamp = opt_datetime_to_timestamp(Some(now));
        let dt_back = opt_timestamp_to_datetime(timestamp);
        assert_eq!(now.timestamp(), dt_back.unwrap().timestamp());
    }

    #[test]
    fn test_opt_timestamp_none() {
        let timestamp = opt_datetime_to_timestamp(None);
        assert!(timestamp.is_none());
        let dt = opt_timestamp_to_datetime(None);
        assert!(dt.is_none());
    }
}
