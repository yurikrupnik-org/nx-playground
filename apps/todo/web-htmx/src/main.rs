//! Todo hypermedia frontend: axum + htmx.
//!
//! The third variant of the same UI (Solid island / Astro+htmx / this):
//! identical DOM contract, but here axum renders the HTML fragments itself —
//! no JS runtime on the server. Talks to todo-api over JSON exactly like the
//! Astro variant; ships zero app JS to the browser (htmx only, self-hosted).
//!
//! Single static binary: theme CSS and htmx.min.js are embedded at compile
//! time (`assets/htmx.min.js` is vendored from htmx.org@2.0.10 — bun installs
//! node_modules app-locally, which doesn't exist in cargo/docker builds).

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Form, Router};
use axum_helpers::create_app;
use core_config::{app_info, FromEnv};
use eyre::{Result, WrapErr};
use serde::Deserialize;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

use crate::api::{CreateTodo, Priority, TodoApi};
use crate::config::AppConfig;
use crate::fragments::{render_page, render_todo_list};

mod api;
mod config;
mod fragments;

const THEME_CSS: &str = include_str!("../../../../libs/ui/todo-theme/todo.css");
const HTMX_JS: &str = include_str!("../assets/htmx.min.js");

#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::from_env()?;
    let _tracing_guard = core_config::tracing::init_tracing(&config.environment, app_info!());

    let api = TodoApi::new(config.todo_api_url.clone());
    info!(todo_api_url = %config.todo_api_url, "rendering fragments over todo-api");

    let app = Router::new()
        .route("/", get(index))
        .route("/healthz", get(|| async { "ok" }))
        .route("/assets/todo.css", get(theme_css))
        .route("/assets/htmx.min.js", get(htmx_js))
        .route("/partials/todos", get(list_todos).post(create_todo))
        .route("/partials/todos/{id}", delete(delete_todo))
        .route("/partials/todos/{id}/toggle", post(toggle_todo))
        .with_state(api)
        .layer(TraceLayer::new_for_http());

    info!(addr = %config.server.addr(), "todo-web-htmx listening");
    create_app(app, &config.server)
        .await
        .wrap_err("server error")?;

    Ok(())
}

/// Standard fragment response (htmx swaps on 2xx only).
fn fragment(html: String, status: StatusCode) -> Response {
    (status, Html(html)).into_response()
}

/// Upstream failure: non-2xx text so htmx leaves the current DOM intact.
fn upstream_error(err: eyre::Report) -> Response {
    error!(error = %err, "todo-api call failed");
    (StatusCode::BAD_GATEWAY, "todo-api unavailable").into_response()
}

/// Full page; initial list server-rendered, load failure degrades to an alert.
async fn index(State(api): State<TodoApi>) -> Html<String> {
    let list_html = match api.list().await {
        Ok(todos) => Some(render_todo_list(&todos)),
        Err(err) => {
            error!(error = %err, "initial list failed");
            None
        }
    };
    Html(render_page(list_html.as_deref()))
}

async fn theme_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        THEME_CSS,
    )
}

async fn htmx_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        HTMX_JS,
    )
}

async fn render_list(api: &TodoApi) -> Response {
    match api.list().await {
        Ok(todos) => fragment(render_todo_list(&todos), StatusCode::OK),
        Err(err) => upstream_error(err),
    }
}

async fn list_todos(State(api): State<TodoApi>) -> Response {
    render_list(&api).await
}

/// The form-encoded body htmx submits (title, priority).
#[derive(Debug, Deserialize)]
struct CreateForm {
    #[serde(default)]
    title: String,
    #[serde(default)]
    priority: Option<String>,
}

async fn create_todo(State(api): State<TodoApi>, Form(form): Form<CreateForm>) -> Response {
    let title = form.title.trim();
    let priority = match form.priority.as_deref() {
        None => Some(Priority::Medium),
        Some(value) => Priority::parse(value),
    };
    let (Some(priority), false) = (priority, title.is_empty()) else {
        return fragment("invalid todo".into(), StatusCode::UNPROCESSABLE_ENTITY);
    };

    let input = CreateTodo {
        title: title.to_owned(),
        description: String::new(),
        priority,
    };
    if let Err(err) = api.create(&input).await {
        return upstream_error(err);
    }
    render_list(&api).await
}

/// Flip completion; current state is read from the API so the toggle is
/// idempotent per rendered state.
async fn toggle_todo(State(api): State<TodoApi>, Path(id): Path<String>) -> Response {
    let todo = match api.get(&id).await {
        Ok(Some(todo)) => todo,
        Ok(None) => return fragment("unknown todo".into(), StatusCode::NOT_FOUND),
        Err(err) => return upstream_error(err),
    };
    if let Err(err) = api.toggle(&id, todo.completed).await {
        return upstream_error(err);
    }
    render_list(&api).await
}

async fn delete_todo(State(api): State<TodoApi>, Path(id): Path<String>) -> Response {
    if let Err(err) = api.remove(&id).await {
        return upstream_error(err);
    }
    render_list(&api).await
}
