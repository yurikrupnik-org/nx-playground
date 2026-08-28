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
    info(
        title = "Todos API",
        version = "1.0.0",
        description = "Todo CRUD + lifecycle REST API (served by todo-api under /api/todos)"
    ),
    servers((url = "/api/todos", description = "todo-api mount path")),
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

#[cfg(test)]
mod tests {
    use utoipa::OpenApi;

    /// Regenerates the committed OpenAPI v1 document. Same convention as the
    /// ts-rs `export_bindings_*` tests: running the suite keeps
    /// `docs/openapi/todos.v1.json` in sync with the handler annotations.
    #[test]
    fn export_openapi_todos_v1() {
        let json = super::TodoApiDoc::openapi()
            .to_pretty_json()
            .expect("serialize todos openapi doc");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../docs/openapi/todos.v1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, json + "\n").unwrap();
    }
}
