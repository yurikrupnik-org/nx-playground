//! Task-specific proto ↔ domain conversions
//!
//! This module contains conversions specific to the tasks domain:
//! - TaskPriority ↔ protobuf Priority enum
//! - TaskStatus ↔ protobuf Status enum
//! - Task structs ↔ protobuf message types
//!
//! Generic conversions (UUIDs, timestamps) are re-exported from grpc_client::conversions
//! and shared across all domains (tasks, users, projects, etc.)

use rpc::tasks::{
    CreateRequest, CreateResponse, GetByIdResponse, ListResponse, ListStreamResponse, Priority,
    Status, UpdateByIdRequest, UpdateByIdResponse,
};

use crate::models::{CreateTask, Task, TaskPriority, TaskStatus, UpdateTask};

// Re-export generic proto conversion helpers from shared library
// These are domain-agnostic and used across all services
pub use grpc_client::conversions::*;

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
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match Priority::try_from(value) {
            Ok(Priority::Low) => Ok(TaskPriority::Low),
            Ok(Priority::Medium) => Ok(TaskPriority::Medium),
            Ok(Priority::High) => Ok(TaskPriority::High),
            Ok(Priority::Urgent) => Ok(TaskPriority::Urgent),
            Ok(Priority::Unspecified) | Err(_) => Err(format!("Invalid priority: {}", value)),
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
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match Status::try_from(value) {
            Ok(Status::Todo) => Ok(TaskStatus::Todo),
            Ok(Status::InProgress) => Ok(TaskStatus::InProgress),
            Ok(Status::Done) => Ok(TaskStatus::Done),
            Ok(Status::Unspecified) | Err(_) => Err(format!("Invalid status: {}", value)),
        }
    }
}

// ============================================================================
// Struct Conversions: Domain → Proto (Request types)
// ============================================================================

impl From<CreateTask> for CreateRequest {
    fn from(input: CreateTask) -> Self {
        CreateRequest {
            title: input.title,
            description: input.description,
            project_id: opt_uuid_to_bytes(input.project_id),
            priority: input.priority.into(),
            status: input.status.into(),
            due_date: opt_datetime_to_timestamp(input.due_date),
        }
    }
}

impl From<UpdateTask> for UpdateByIdRequest {
    fn from(input: UpdateTask) -> Self {
        UpdateByIdRequest {
            id: vec![], // Will be set by caller with the actual UUID
            title: input.title,
            description: input.description,
            completed: None, // derived from status, not stored
            project_id: input.project_id.and_then(opt_uuid_to_bytes),
            priority: input.priority.map(Into::into),
            status: input.status.map(Into::into),
            due_date: input.due_date.and_then(opt_datetime_to_timestamp),
        }
    }
}

// ============================================================================
// Struct Conversions: Proto → Domain (Request types - for gRPC server)
// ============================================================================

impl TryFrom<CreateRequest> for CreateTask {
    type Error = String;

    fn try_from(proto: CreateRequest) -> Result<Self, Self::Error> {
        Ok(CreateTask {
            title: proto.title,
            description: proto.description,
            project_id: opt_bytes_to_uuid(proto.project_id)?,
            priority: proto.priority.try_into()?,
            status: proto.status.try_into()?,
            due_date: opt_timestamp_to_datetime(proto.due_date),
        })
    }
}

impl TryFrom<UpdateByIdRequest> for UpdateTask {
    type Error = String;

    fn try_from(proto: UpdateByIdRequest) -> Result<Self, Self::Error> {
        // UpdateTask uses Option<Option<T>> for partial updates
        // None = field not provided, Some(None) = set to null, Some(Some(value)) = set value
        Ok(UpdateTask {
            title: proto.title,
            description: proto.description,
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

impl TryFrom<CreateResponse> for Task {
    type Error = String;

    fn try_from(proto: CreateResponse) -> Result<Self, Self::Error> {
        Ok(Task {
            id: bytes_to_uuid(&proto.id)?,
            title: proto.title,
            description: proto.description,
            project_id: opt_bytes_to_uuid(proto.project_id).ok().flatten(),
            priority: proto.priority.try_into().unwrap_or_else(|e| {
                tracing::warn!("Invalid priority in CreateResponse, defaulting: {e}");
                Default::default()
            }),
            status: proto.status.try_into().unwrap_or_else(|e| {
                tracing::warn!("Invalid status in CreateResponse, defaulting: {e}");
                Default::default()
            }),
            due_date: opt_timestamp_to_datetime(proto.due_date),
            created_at: timestamp_to_datetime(proto.created_at),
            updated_at: timestamp_to_datetime(proto.updated_at),
        })
    }
}

impl TryFrom<GetByIdResponse> for Task {
    type Error = String;

    fn try_from(proto: GetByIdResponse) -> Result<Self, Self::Error> {
        Ok(Task {
            id: bytes_to_uuid(&proto.id)?,
            title: proto.title,
            description: proto.description,
            project_id: opt_bytes_to_uuid(proto.project_id).ok().flatten(),
            priority: proto.priority.try_into().unwrap_or_else(|e| {
                tracing::warn!("Invalid priority in GetByIdResponse, defaulting: {e}");
                Default::default()
            }),
            status: proto.status.try_into().unwrap_or_else(|e| {
                tracing::warn!("Invalid status in GetByIdResponse, defaulting: {e}");
                Default::default()
            }),
            due_date: opt_timestamp_to_datetime(proto.due_date),
            created_at: timestamp_to_datetime(proto.created_at),
            updated_at: timestamp_to_datetime(proto.updated_at),
        })
    }
}

impl TryFrom<UpdateByIdResponse> for Task {
    type Error = String;

    fn try_from(proto: UpdateByIdResponse) -> Result<Self, Self::Error> {
        Ok(Task {
            id: bytes_to_uuid(&proto.id)?,
            title: proto.title,
            description: proto.description,
            project_id: opt_bytes_to_uuid(proto.project_id).ok().flatten(),
            priority: proto.priority.try_into().unwrap_or_else(|e| {
                tracing::warn!("Invalid priority in UpdateByIdResponse, defaulting: {e}");
                Default::default()
            }),
            status: proto.status.try_into().unwrap_or_else(|e| {
                tracing::warn!("Invalid status in UpdateByIdResponse, defaulting: {e}");
                Default::default()
            }),
            due_date: opt_timestamp_to_datetime(proto.due_date),
            created_at: timestamp_to_datetime(proto.created_at),
            updated_at: timestamp_to_datetime(proto.updated_at),
        })
    }
}

// ============================================================================
// Struct Conversions: Domain → Proto (Response types - for gRPC server)
// ============================================================================

impl From<Task> for CreateResponse {
    fn from(task: Task) -> Self {
        CreateResponse {
            id: uuid_to_bytes(task.id),
            title: task.title,
            description: task.description,
            completed: task.status == TaskStatus::Done,
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
            title: task.title,
            description: task.description,
            completed: task.status == TaskStatus::Done,
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
            title: task.title,
            description: task.description,
            completed: task.status == TaskStatus::Done,
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
            title: task.title,
            description: task.description,
            completed: task.status == TaskStatus::Done,
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

// Helper function for ListResponse conversion (can't implement TryFrom due to orphan rules)
pub fn list_response_to_tasks(proto: ListResponse) -> Result<Vec<Task>, String> {
    proto.data.into_iter().map(|item| item.try_into()).collect()
}
