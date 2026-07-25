//! Task-specific proto ↔ domain conversions
//!
//! This module contains conversions specific to the tasks domain:
//! - TaskPriority ↔ protobuf Priority enum
//! - TaskStatus ↔ protobuf Status enum
//! - Task structs ↔ protobuf message types
//!
//! Generic conversions (UUIDs, timestamps) are named re-exports from
//! `grpc_client::conversions` and shared across all domains.

use rpc::tasks::v1::{
    CreateRequest, CreateResponse, GetByIdResponse, ListResponse, ListStreamResponse, Priority,
    Status, UpdateByIdRequest, UpdateByIdResponse,
};
use uuid::Uuid;

use crate::models::{CreateTask, Task, TaskPriority, TaskScope, TaskStatus, UpdateTask};

// Named re-exports of the domain-agnostic proto helpers. A glob (`*`) would
// leak the entire foreign helper surface into this crate's public API.
pub use grpc_client::conversions::{
    bytes_to_uuid, datetime_to_timestamp, opt_bytes_to_uuid, opt_datetime_to_timestamp,
    opt_timestamp_to_datetime, opt_uuid_to_bytes, timestamp_to_datetime, uuid_to_bytes,
};

/// Errors converting between proto messages and domain [`Task`] values.
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    /// Raw bytes did not decode into a valid UUID.
    #[error("invalid UUID: {0}")]
    BadUuid(String),
    /// Proto enum discriminant is unknown/unspecified.
    #[error("invalid enum discriminant: {0}")]
    BadEnum(i32),
}

impl From<ConversionError> for tonic::Status {
    fn from(err: ConversionError) -> Self {
        tonic::Status::invalid_argument(err.to_string())
    }
}

// ============================================================================
// Priority Conversions
// ============================================================================

impl From<TaskPriority> for i32 {
    fn from(priority: TaskPriority) -> Self {
        match priority {
            TaskPriority::Low => Priority::Low as i32,
            TaskPriority::Medium => Priority::Medium as i32,
            TaskPriority::High => Priority::High as i32,
            TaskPriority::Urgent => Priority::Urgent as i32,
        }
    }
}

impl From<&TaskPriority> for i32 {
    fn from(priority: &TaskPriority) -> Self {
        (*priority).into()
    }
}

impl TryFrom<i32> for TaskPriority {
    type Error = ConversionError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match Priority::try_from(value) {
            Ok(Priority::Low) => Ok(TaskPriority::Low),
            Ok(Priority::Medium) => Ok(TaskPriority::Medium),
            Ok(Priority::High) => Ok(TaskPriority::High),
            Ok(Priority::Urgent) => Ok(TaskPriority::Urgent),
            Ok(Priority::Unspecified) | Err(_) => Err(ConversionError::BadEnum(value)),
        }
    }
}

// ============================================================================
// Status Conversions
// ============================================================================

impl From<TaskStatus> for i32 {
    fn from(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Todo => Status::Todo as i32,
            TaskStatus::InProgress => Status::InProgress as i32,
            TaskStatus::Done => Status::Done as i32,
        }
    }
}

impl From<&TaskStatus> for i32 {
    fn from(status: &TaskStatus) -> Self {
        (*status).into()
    }
}

impl TryFrom<i32> for TaskStatus {
    type Error = ConversionError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match Status::try_from(value) {
            Ok(Status::Todo) => Ok(TaskStatus::Todo),
            Ok(Status::InProgress) => Ok(TaskStatus::InProgress),
            Ok(Status::Done) => Ok(TaskStatus::Done),
            Ok(Status::Unspecified) | Err(_) => Err(ConversionError::BadEnum(value)),
        }
    }
}

// ============================================================================
// Struct Conversions: Domain → Proto (Request types)
// ============================================================================

/// Build a [`CreateRequest`] from the BFF-resolved tenant scope + client payload.
/// A plain `From<CreateTask>` would leave the scope fields empty for the caller
/// to backfill — same footgun as the retired `From<UpdateTask>` (see below).
pub fn make_create_request(scope: TaskScope, input: CreateTask) -> CreateRequest {
    CreateRequest {
        title: input.title,
        description: input.description,
        project_id: opt_uuid_to_bytes(input.project_id),
        priority: input.priority.into(),
        status: input.status.into(),
        due_date: opt_datetime_to_timestamp(input.due_date),
        org_id: uuid_to_bytes(scope.org_id),
        user_id: uuid_to_bytes(scope.user_id),
    }
}

/// Build an [`UpdateByIdRequest`] for `id` from a partial [`UpdateTask`].
///
/// Replaces the former `From<UpdateTask> for UpdateByIdRequest`, which left
/// `id: vec![]` for the caller to backfill — a footgun that let a forgotten
/// assignment produce a type-checked but empty-id request.
pub fn make_update_request(org_id: Uuid, id: Uuid, input: UpdateTask) -> UpdateByIdRequest {
    UpdateByIdRequest {
        id: uuid_to_bytes(id),
        org_id: uuid_to_bytes(org_id),
        title: input.title,
        description: input.description,
        completed: input.completed,
        project_id: input.project_id.and_then(opt_uuid_to_bytes),
        priority: input.priority.map(Into::into),
        status: input.status.map(Into::into),
        due_date: input.due_date.and_then(opt_datetime_to_timestamp),
    }
}

// ============================================================================
// Struct Conversions: Proto → Domain (Request types - for gRPC server)
// ============================================================================

impl TryFrom<CreateRequest> for CreateTask {
    type Error = ConversionError;

    fn try_from(proto: CreateRequest) -> Result<Self, Self::Error> {
        Ok(CreateTask {
            title: proto.title,
            description: proto.description,
            project_id: opt_bytes_to_uuid(proto.project_id).map_err(ConversionError::BadUuid)?,
            priority: proto.priority.try_into()?,
            status: proto.status.try_into()?,
            due_date: opt_timestamp_to_datetime(proto.due_date),
        })
    }
}

impl TryFrom<UpdateByIdRequest> for UpdateTask {
    type Error = ConversionError;

    fn try_from(proto: UpdateByIdRequest) -> Result<Self, Self::Error> {
        // UpdateTask uses Option<Option<T>> for partial updates
        // None = field not provided, Some(None) = set to null, Some(Some(value)) = set value
        Ok(UpdateTask {
            title: proto.title,
            description: proto.description,
            completed: proto.completed,
            project_id: proto.project_id.map(|bytes| bytes_to_uuid(&bytes).ok()),
            priority: proto.priority.map(|p| p.try_into()).transpose()?,
            status: proto.status.map(|s| s.try_into()).transpose()?,
            due_date: proto.due_date.map(|ts| Some(timestamp_to_datetime(ts))),
        })
    }
}

// ============================================================================
// Struct Conversions: Proto → Domain (Response types - for gRPC client)
// ============================================================================

/// Shared body for the three byte-identical `TryFrom<*Response> for Task`
/// impls. Invalid enum discriminants degrade to the type default (logged),
/// matching the pre-existing best-effort client behavior; only a malformed id
/// is fatal.
#[allow(clippy::too_many_arguments)]
fn task_from_response(
    id: Vec<u8>,
    org_id: Vec<u8>,
    user_id: Vec<u8>,
    title: String,
    description: String,
    completed: bool,
    project_id: Option<Vec<u8>>,
    priority: i32,
    status: i32,
    due_date: Option<i64>,
    created_at: i64,
    updated_at: i64,
) -> Result<Task, ConversionError> {
    Ok(Task {
        id: bytes_to_uuid(&id).map_err(ConversionError::BadUuid)?,
        org_id: bytes_to_uuid(&org_id).map_err(ConversionError::BadUuid)?,
        user_id: bytes_to_uuid(&user_id).map_err(ConversionError::BadUuid)?,
        title,
        description,
        completed,
        project_id: opt_bytes_to_uuid(project_id).ok().flatten(),
        priority: priority.try_into().unwrap_or_else(|e| {
            tracing::warn!("Invalid priority in response, defaulting: {e}");
            Default::default()
        }),
        status: status.try_into().unwrap_or_else(|e| {
            tracing::warn!("Invalid status in response, defaulting: {e}");
            Default::default()
        }),
        due_date: opt_timestamp_to_datetime(due_date),
        created_at: timestamp_to_datetime(created_at),
        updated_at: timestamp_to_datetime(updated_at),
    })
}

macro_rules! task_try_from_response {
    ($($resp:ty),+ $(,)?) => {$(
        impl TryFrom<$resp> for Task {
            type Error = ConversionError;

            fn try_from(proto: $resp) -> Result<Self, Self::Error> {
                task_from_response(
                    proto.id,
                    proto.org_id,
                    proto.user_id,
                    proto.title,
                    proto.description,
                    proto.completed,
                    proto.project_id,
                    proto.priority,
                    proto.status,
                    proto.due_date,
                    proto.created_at,
                    proto.updated_at,
                )
            }
        }
    )+};
}

task_try_from_response!(CreateResponse, GetByIdResponse, UpdateByIdResponse);

// ============================================================================
// Struct Conversions: Domain → Proto (Response types - for gRPC server)
// ============================================================================

impl From<Task> for CreateResponse {
    fn from(task: Task) -> Self {
        CreateResponse {
            id: uuid_to_bytes(task.id),
            org_id: uuid_to_bytes(task.org_id),
            user_id: uuid_to_bytes(task.user_id),
            title: task.title,
            description: task.description,
            completed: task.completed,
            project_id: opt_uuid_to_bytes(task.project_id),
            priority: task.priority.into(),
            status: task.status.into(),
            due_date: opt_datetime_to_timestamp(task.due_date),
            created_at: datetime_to_timestamp(task.created_at),
            updated_at: datetime_to_timestamp(task.updated_at),
        }
    }
}

impl From<Task> for GetByIdResponse {
    fn from(task: Task) -> Self {
        GetByIdResponse {
            id: uuid_to_bytes(task.id),
            org_id: uuid_to_bytes(task.org_id),
            user_id: uuid_to_bytes(task.user_id),
            title: task.title,
            description: task.description,
            completed: task.completed,
            project_id: opt_uuid_to_bytes(task.project_id),
            priority: task.priority.into(),
            status: task.status.into(),
            due_date: opt_datetime_to_timestamp(task.due_date),
            created_at: datetime_to_timestamp(task.created_at),
            updated_at: datetime_to_timestamp(task.updated_at),
        }
    }
}

impl From<Task> for UpdateByIdResponse {
    fn from(task: Task) -> Self {
        UpdateByIdResponse {
            id: uuid_to_bytes(task.id),
            org_id: uuid_to_bytes(task.org_id),
            user_id: uuid_to_bytes(task.user_id),
            title: task.title,
            description: task.description,
            completed: task.completed,
            project_id: opt_uuid_to_bytes(task.project_id),
            priority: task.priority.into(),
            status: task.status.into(),
            due_date: opt_datetime_to_timestamp(task.due_date),
            created_at: datetime_to_timestamp(task.created_at),
            updated_at: datetime_to_timestamp(task.updated_at),
        }
    }
}

impl From<Task> for ListStreamResponse {
    fn from(task: Task) -> Self {
        ListStreamResponse {
            id: uuid_to_bytes(task.id),
            org_id: uuid_to_bytes(task.org_id),
            user_id: uuid_to_bytes(task.user_id),
            title: task.title,
            description: task.description,
            completed: task.completed,
            project_id: opt_uuid_to_bytes(task.project_id),
            priority: task.priority.into(),
            status: task.status.into(),
            due_date: opt_datetime_to_timestamp(task.due_date),
            created_at: datetime_to_timestamp(task.created_at),
            updated_at: datetime_to_timestamp(task.updated_at),
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert a `ListResponse` into domain tasks (orphan rules preclude a `TryFrom`).
pub fn list_response_to_tasks(proto: ListResponse) -> Result<Vec<Task>, ConversionError> {
    proto.data.into_iter().map(|item| item.try_into()).collect()
}
