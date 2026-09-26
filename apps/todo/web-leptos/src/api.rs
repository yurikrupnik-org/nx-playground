//! JSON client for todo-api — the Rust twin of
//! `apps/todo/web/src/lib/todo-api.ts`.
//!
//! Same-origin `/api/todos`, so the nginx/caddy `/api` proxy the Solid image
//! already ships works unchanged and no origin is baked into the wasm.

use gloo_net::http::Request;

use crate::dto::{CreateTodo, Todo, UpdateTodo};
use crate::identity::TODO_APP;

const API_BASE_URL: &str = "/api/todos";

/// Anything a mutation can fail with, already phrased for the UI.
pub type ApiResult<T> = Result<T, String>;

/// Turn a failed response into an error the UI can show verbatim.
///
/// todo-api enforces the flag catalogue server-side, so the two flag statuses
/// get their own messages instead of a generic "failed to …".
fn request_error(status: u16, fallback: &str) -> String {
    match status {
        403 => "writes are disabled by feature flag (todo_write)".to_owned(),
        429 => "todo limit reached (todo_max_items)".to_owned(),
        _ => fallback.to_owned(),
    }
}

/// Identity headers every todo-api request must carry so flags resolve per
/// user/app. `Content-Type` is added by `RequestBuilder::json`.
fn with_identity(builder: gloo_net::http::RequestBuilder, identity: &str) -> gloo_net::http::RequestBuilder {
    builder
        .header("X-Todo-Identity", identity)
        .header("X-Todo-App", TODO_APP)
}

pub async fn list(identity: &str) -> ApiResult<Vec<Todo>> {
    let response = with_identity(
        Request::get(&format!("{API_BASE_URL}?limit=100000")),
        identity,
    )
    .send()
    .await
    .map_err(|_| "Failed to fetch todos".to_owned())?;

    if !response.ok() {
        return Err(request_error(response.status(), "Failed to fetch todos"));
    }
    response
        .json()
        .await
        .map_err(|_| "Failed to fetch todos".to_owned())
}

pub async fn create(identity: &str, input: &CreateTodo) -> ApiResult<Todo> {
    let response = with_identity(Request::post(API_BASE_URL), identity)
        .json(input)
        .map_err(|_| "Failed to create todo".to_owned())?
        .send()
        .await
        .map_err(|_| "Failed to create todo".to_owned())?;

    if !response.ok() {
        return Err(request_error(response.status(), "Failed to create todo"));
    }
    response
        .json()
        .await
        .map_err(|_| "Failed to create todo".to_owned())
}

/// `PUT /api/todos/{id}`. Mirrors `todoApi.update`; the `/` route never calls
/// it (a toggle goes through [`complete`]/[`uncomplete`]).
#[allow(dead_code)]
pub async fn update(identity: &str, id: &str, input: &UpdateTodo) -> ApiResult<Todo> {
    let response = with_identity(Request::put(&format!("{API_BASE_URL}/{id}")), identity)
        .json(input)
        .map_err(|_| "Failed to update todo".to_owned())?
        .send()
        .await
        .map_err(|_| "Failed to update todo".to_owned())?;

    if !response.ok() {
        return Err(request_error(response.status(), "Failed to update todo"));
    }
    response
        .json()
        .await
        .map_err(|_| "Failed to update todo".to_owned())
}

async fn transition(identity: &str, id: &str, verb: &str, fallback: &str) -> ApiResult<Todo> {
    let response = with_identity(
        Request::post(&format!("{API_BASE_URL}/{id}/{verb}")),
        identity,
    )
    .send()
    .await
    .map_err(|_| fallback.to_owned())?;

    if !response.ok() {
        return Err(request_error(response.status(), fallback));
    }
    response.json().await.map_err(|_| fallback.to_owned())
}

pub async fn complete(identity: &str, id: &str) -> ApiResult<Todo> {
    transition(identity, id, "complete", "Failed to complete todo").await
}

pub async fn uncomplete(identity: &str, id: &str) -> ApiResult<Todo> {
    transition(identity, id, "uncomplete", "Failed to uncomplete todo").await
}

pub async fn remove(identity: &str, id: &str) -> ApiResult<()> {
    let response = with_identity(Request::delete(&format!("{API_BASE_URL}/{id}")), identity)
        .send()
        .await
        .map_err(|_| "Failed to delete todo".to_owned())?;

    if !response.ok() {
        return Err(request_error(response.status(), "Failed to delete todo"));
    }
    Ok(())
}
