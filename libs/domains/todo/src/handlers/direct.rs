//! Axum HTTP handlers for the Todo REST API (no auth — standalone service).

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use axum_helpers::UuidPath;

use crate::error::TodoResult;
use crate::models::{CreateTodo, Todo, TodoFilter, UpdateTodo};
use crate::repository::TodoRepository;
use crate::service::TodoService;

#[utoipa::path(
    get,
    path = "/",
    params(TodoFilter),
    responses((status = 200, description = "List todos", body = [Todo])),
    tag = "todos"
)]
pub async fn list_todos<R: TodoRepository>(
    State(service): State<Arc<TodoService<R>>>,
    Query(filter): Query<TodoFilter>,
) -> TodoResult<Json<Vec<Todo>>> {
    Ok(Json(service.list_todos(filter).await?))
}

#[utoipa::path(
    get,
    path = "/{id}",
    responses((status = 200, description = "Get a todo", body = Todo)),
    tag = "todos"
)]
pub async fn get_todo<R: TodoRepository>(
    State(service): State<Arc<TodoService<R>>>,
    UuidPath(id): UuidPath,
) -> TodoResult<Json<Todo>> {
    Ok(Json(service.get_todo(id).await?))
}

#[utoipa::path(
    post,
    path = "/",
    request_body = CreateTodo,
    responses((status = 201, description = "Created", body = Todo)),
    tag = "todos"
)]
pub async fn create_todo<R: TodoRepository>(
    State(service): State<Arc<TodoService<R>>>,
    Json(input): Json<CreateTodo>,
) -> TodoResult<impl IntoResponse> {
    let todo = service.create_todo(input).await?;
    Ok((StatusCode::CREATED, Json(todo)))
}

#[utoipa::path(
    put,
    path = "/{id}",
    request_body = UpdateTodo,
    responses((status = 200, description = "Updated", body = Todo)),
    tag = "todos"
)]
pub async fn update_todo<R: TodoRepository>(
    State(service): State<Arc<TodoService<R>>>,
    UuidPath(id): UuidPath,
    Json(input): Json<UpdateTodo>,
) -> TodoResult<Json<Todo>> {
    Ok(Json(service.update_todo(id, input).await?))
}

#[utoipa::path(
    delete,
    path = "/{id}",
    responses((status = 204, description = "Deleted")),
    tag = "todos"
)]
pub async fn delete_todo<R: TodoRepository>(
    State(service): State<Arc<TodoService<R>>>,
    UuidPath(id): UuidPath,
) -> TodoResult<impl IntoResponse> {
    service.delete_todo(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/{id}/complete",
    responses((status = 200, description = "Marked complete", body = Todo)),
    tag = "todos"
)]
pub async fn complete_todo<R: TodoRepository>(
    State(service): State<Arc<TodoService<R>>>,
    UuidPath(id): UuidPath,
) -> TodoResult<Json<Todo>> {
    Ok(Json(service.complete_todo(id).await?))
}

#[utoipa::path(
    post,
    path = "/{id}/uncomplete",
    responses((status = 200, description = "Marked incomplete", body = Todo)),
    tag = "todos"
)]
pub async fn uncomplete_todo<R: TodoRepository>(
    State(service): State<Arc<TodoService<R>>>,
    UuidPath(id): UuidPath,
) -> TodoResult<Json<Todo>> {
    Ok(Json(service.uncomplete_todo(id).await?))
}
