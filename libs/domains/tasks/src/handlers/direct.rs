use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::TaskResult;
use crate::models::{CreateTask, Task, TaskFilter, UpdateTask};
use crate::repository::TaskRepository;
use crate::service::TaskService;

/// List tasks with optional filters
#[utoipa::path(
    get,
    path = "",
    tag = "tasks-direct",
    responses(
        (status = 200, description = "List of tasks", body = Vec<Task>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_tasks<R: TaskRepository>(
    State(service): State<Arc<TaskService<R>>>,
) -> TaskResult<Json<Vec<Task>>> {
    let filter = TaskFilter {
        project_id: None,
        status: None,
        priority: None,
        limit: 50,
        offset: 0,
    };

    let tasks = service.list_tasks(filter).await?;
    Ok(Json(tasks))
}

/// Get a task by ID
#[utoipa::path(
    get,
    path = "/{id}",
    tag = "tasks-direct",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    responses(
        (status = 200, description = "Task found", body = Task),
        (status = 400, description = "Invalid task ID"),
        (status = 404, description = "Task not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_task<R: TaskRepository>(
    State(service): State<Arc<TaskService<R>>>,
    Path(id): Path<String>,
) -> TaskResult<impl IntoResponse> {
    let task_id = Uuid::parse_str(&id)
        .map_err(|_| crate::error::TaskError::Validation("Invalid task ID".to_string()))?;

    let task = service.get_task(task_id).await?;
    Ok(Json(task))
}

/// Create a new task
#[utoipa::path(
    post,
    path = "",
    tag = "tasks-direct",
    request_body = CreateTask,
    responses(
        (status = 201, description = "Task created successfully", body = Task),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn create_task<R: TaskRepository>(
    State(service): State<Arc<TaskService<R>>>,
    Json(input): Json<CreateTask>,
) -> TaskResult<impl IntoResponse> {
    let task = service.create_task(input).await?;
    Ok((StatusCode::CREATED, Json(task)))
}

/// Update a task
#[utoipa::path(
    put,
    path = "/{id}",
    tag = "tasks-direct",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    request_body = UpdateTask,
    responses(
        (status = 200, description = "Task updated successfully", body = Task),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Task not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_task<R: TaskRepository>(
    State(service): State<Arc<TaskService<R>>>,
    Path(id): Path<String>,
    Json(input): Json<UpdateTask>,
) -> TaskResult<impl IntoResponse> {
    let task_id = Uuid::parse_str(&id)
        .map_err(|_| crate::error::TaskError::Validation("Invalid task ID".to_string()))?;

    let task = service.update_task(task_id, input).await?;
    Ok(Json(task))
}

/// Delete a task
#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "tasks-direct",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    responses(
        (status = 204, description = "Task deleted successfully"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Task not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_task<R: TaskRepository>(
    State(service): State<Arc<TaskService<R>>>,
    Path(id): Path<String>,
) -> TaskResult<impl IntoResponse> {
    let task_id = Uuid::parse_str(&id)
        .map_err(|_| crate::error::TaskError::Validation("Invalid task ID".to_string()))?;

    service.delete_task(task_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
