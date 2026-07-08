use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rpc::tasks::tasks_service_client::TasksServiceClient;
use rpc::tasks::{DeleteByIdRequest, GetByIdRequest, ListRequest, UpdateByIdRequest};
use grpc_client::TracedChannel;
use uuid::Uuid;

use crate::error::{TaskError, TaskResult};
use crate::models::{CreateTask, Task, UpdateTask};

// Import proto conversion helpers
use crate::conversions::*;

/// List tasks via gRPC
#[utoipa::path(
    get,
    path = "",
    tag = "tasks",
    responses(
        (status = 200, description = "List of tasks via gRPC", body = Vec<Task>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_tasks(
    State(mut client): State<TasksServiceClient<TracedChannel>>,
) -> TaskResult<Json<Vec<Task>>> {
    let response = client
        .list(ListRequest {
            project_id: None,
            status: None,
            priority: None,
            completed: None,
            limit: 50,
            offset: 0,
        })
        .await
        .map_err(|e| TaskError::Internal(e.to_string()))?;

    let tasks = list_response_to_tasks(response.into_inner())
        .map_err(|e| TaskError::Internal(format!("Conversion error: {}", e)))?;

    Ok(Json(tasks))
}

/// Get a task by ID via gRPC
#[utoipa::path(
    get,
    path = "/{id}",
    tag = "tasks",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    responses(
        (status = 200, description = "Task found via gRPC", body = Task),
        (status = 400, description = "Invalid task ID"),
        (status = 404, description = "Task not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_task(
    State(mut client): State<TasksServiceClient<TracedChannel>>,
    Path(id): Path<String>,
) -> TaskResult<impl IntoResponse> {
    let uuid =
        Uuid::parse_str(&id).map_err(|_| TaskError::Validation("Invalid task ID".to_string()))?;

    let response = client
        .get_by_id(GetByIdRequest {
            id: uuid_to_bytes(uuid),
        })
        .await
        .map_err(|e| {
            if e.code() == tonic::Code::NotFound {
                TaskError::NotFound(uuid)
            } else {
                TaskError::Internal(e.to_string())
            }
        })?;

    let task: Task = response
        .into_inner()
        .try_into()
        .map_err(|e| TaskError::Internal(format!("Conversion error: {}", e)))?;

    Ok(Json(task))
}

/// Create a new task via gRPC
#[utoipa::path(
    post,
    path = "",
    tag = "tasks",
    request_body = CreateTask,
    responses(
        (status = 201, description = "Task created successfully via gRPC", body = Task),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn create_task(
    State(mut client): State<TasksServiceClient<TracedChannel>>,
    Json(input): Json<CreateTask>,
) -> TaskResult<impl IntoResponse> {
    let response = client
        .create(rpc::tasks::CreateRequest::from(input))
        .await
        .map_err(|e| TaskError::Internal(e.to_string()))?;

    let task: Task = response
        .into_inner()
        .try_into()
        .map_err(|e| TaskError::Internal(format!("Conversion error: {}", e)))?;

    Ok((StatusCode::CREATED, Json(task)))
}

/// Update a task via gRPC
#[utoipa::path(
    put,
    path = "/{id}",
    tag = "tasks",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    request_body = UpdateTask,
    responses(
        (status = 200, description = "Task updated successfully via gRPC", body = Task),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Task not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_task(
    State(mut client): State<TasksServiceClient<TracedChannel>>,
    Path(id): Path<String>,
    Json(input): Json<UpdateTask>,
) -> TaskResult<impl IntoResponse> {
    let uuid =
        Uuid::parse_str(&id).map_err(|_| TaskError::Validation("Invalid task ID".to_string()))?;

    let mut request: UpdateByIdRequest = input.into();
    request.id = uuid_to_bytes(uuid);

    let response = client.update_by_id(request).await.map_err(|e| {
        if e.code() == tonic::Code::NotFound {
            TaskError::NotFound(uuid)
        } else {
            TaskError::Internal(e.to_string())
        }
    })?;

    let task: Task = response
        .into_inner()
        .try_into()
        .map_err(|e| TaskError::Internal(format!("Conversion error: {}", e)))?;

    Ok(Json(task))
}

/// Delete a task via gRPC
#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "tasks",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    responses(
        (status = 204, description = "Task deleted successfully via gRPC"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Task not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_task(
    State(mut client): State<TasksServiceClient<TracedChannel>>,
    Path(id): Path<String>,
) -> TaskResult<impl IntoResponse> {
    let uuid =
        Uuid::parse_str(&id).map_err(|_| TaskError::Validation("Invalid task ID".to_string()))?;

    client
        .delete_by_id(DeleteByIdRequest {
            id: uuid_to_bytes(uuid),
        })
        .await
        .map_err(|e| {
            if e.code() == tonic::Code::NotFound {
                TaskError::NotFound(uuid)
            } else {
                TaskError::Internal(e.to_string())
            }
        })?;

    Ok(StatusCode::NO_CONTENT)
}
