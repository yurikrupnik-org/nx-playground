//! HTTP router + OpenAPI doc for the Todo domain.

mod direct;

use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use utoipa::OpenApi;

use crate::models::{CreateTodo, Todo, TodoPriority, UpdateTodo};
use crate::repository::TodoRepository;
use crate::service::TodoService;

pub use direct::{
    complete_todo, create_todo, delete_todo, get_todo, list_todos, uncomplete_todo, update_todo,
};

/// OpenAPI documentation for the Todo REST API.
#[derive(OpenApi)]
#[openapi(
    paths(
        direct::list_todos,
        direct::get_todo,
        direct::create_todo,
        direct::update_todo,
        direct::delete_todo,
        direct::complete_todo,
        direct::uncomplete_todo,
    ),
    components(schemas(Todo, CreateTodo, UpdateTodo, TodoPriority)),
    tags((name = "todos", description = "Todo CRUD + lifecycle operations"))
)]
pub struct TodoApiDoc;

/// Build the todo router mounted by the API binary.
pub fn router<R: TodoRepository + 'static>(service: TodoService<R>) -> Router {
    let shared = Arc::new(service);

    Router::new()
        .route("/", get(list_todos::<R>).post(create_todo::<R>))
        .route(
            "/{id}",
            get(get_todo::<R>)
                .put(update_todo::<R>)
                .delete(delete_todo::<R>),
        )
        .route("/{id}/complete", post(complete_todo::<R>))
        .route("/{id}/uncomplete", post(uncomplete_todo::<R>))
        .with_state(shared)
}
