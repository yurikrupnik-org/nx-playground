//! Axum HTTP handlers for the Todo REST API (no auth — standalone service).

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
// `Uuid` in the `params(("id" = Uuid, Path, ...))` annotations is resolved by
// utoipa from the identifier alone — importing the type would be an unused
// import. The emitted schema is still `{"type":"string","format":"uuid"}`.
use axum_helpers::{ErrorResponse, UuidPath};

use crate::error::TodoResult;
use crate::models::{CreateTodo, Todo, TodoFilter, UpdateTodo};
use crate::repository::TodoRepository;
use crate::service::TodoService;

#[utoipa::path(
    get,
    // `""`, not `"/"`: `utoipa`'s `nest` is a string concat, so `"/"` under a
    // `/todos` mount yields the key `/todos/` — a route axum does not serve and
    // axum 0.8 does not redirect to. The standalone document (`todos.v1.json`,
    // whose `servers[0].url` IS the mount path) rewrites `""` back to `"/"` at
    // export. Same convention as `apps/zerg/api/src/api/tasks.rs`.
    path = "",
    params(TodoFilter),
    responses(
        (status = 200, description = "List todos", body = [Todo]),
        (status = 500, description = "Internal error", body = ErrorResponse),
    ),
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
    params(("id" = Uuid, Path, description = "Todo id")),
    responses(
        (status = 200, description = "Get a todo", body = Todo),
        (status = 400, description = "Malformed id", body = ErrorResponse),
        (status = 404, description = "No such todo", body = ErrorResponse),
    ),
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
    path = "",
    request_body = CreateTodo,
    responses(
        (status = 201, description = "Created", body = Todo),
        (status = 400, description = "Invalid body", body = ErrorResponse),
    ),
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
    params(("id" = Uuid, Path, description = "Todo id")),
    request_body = UpdateTodo,
    responses(
        (status = 200, description = "Updated", body = Todo),
        (status = 400, description = "Invalid body or id", body = ErrorResponse),
        (status = 404, description = "No such todo", body = ErrorResponse),
    ),
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
    params(("id" = Uuid, Path, description = "Todo id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, description = "Malformed id", body = ErrorResponse),
        (status = 404, description = "No such todo", body = ErrorResponse),
    ),
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
    params(("id" = Uuid, Path, description = "Todo id")),
    responses(
        (status = 200, description = "Marked complete", body = Todo),
        (status = 400, description = "Malformed id", body = ErrorResponse),
        (status = 404, description = "No such todo", body = ErrorResponse),
    ),
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
    params(("id" = Uuid, Path, description = "Todo id")),
    responses(
        (status = 200, description = "Marked incomplete", body = Todo),
        (status = 400, description = "Malformed id", body = ErrorResponse),
        (status = 404, description = "No such todo", body = ErrorResponse),
    ),
    tag = "todos"
)]
pub async fn uncomplete_todo<R: TodoRepository>(
    State(service): State<Arc<TodoService<R>>>,
    UuidPath(id): UuidPath,
) -> TodoResult<Json<Todo>> {
    Ok(Json(service.uncomplete_todo(id).await?))
}
