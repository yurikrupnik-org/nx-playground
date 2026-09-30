//! Read-only HTTP surface over the projection. Handlers only look things up:
//! every derived value (trees, self time, estimates) comes from the domain's
//! `Projection` views.

use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Json, Response};
use axum::routing::get;
use axum_helpers::{AppError, UuidPath};
use futures::StreamExt;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tracing::warn;

use crate::projector::Shared;

const UI: &str = include_str!("../ui/index.html");

#[derive(Clone)]
pub struct AppState {
    pub shared: Arc<Shared>,
    pub trace_url: Option<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(|| async { Html(UI) }))
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .route("/api/config", get(config))
        .route("/api/graphs", get(graphs))
        .route("/api/graphs/{id}", get(graph))
        .route("/api/graphs/{id}/tasks/{task}", get(task))
        .route("/api/runs", get(runs))
        .route("/api/runs/{id}", get(run))
        .route("/api/events/sse", get(sse))
        .with_state(state)
}

/// Not ready until the replay has caught up: before that the drill-down is
/// incomplete, and a load balancer should not send anyone to it.
async fn readyz(State(state): State<AppState>) -> Response {
    if state.shared.is_ready() {
        "ready".into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "replaying TASKGRAPH").into_response()
    }
}

#[derive(Serialize)]
struct UiConfig {
    trace_url: Option<String>,
    ready: bool,
}

async fn config(State(state): State<AppState>) -> Json<UiConfig> {
    Json(UiConfig {
        trace_url: state.trace_url.clone(),
        ready: state.shared.is_ready(),
    })
}

async fn graphs(State(state): State<AppState>) -> Response {
    Json(state.shared.projection.read().graphs()).into_response()
}

async fn graph(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let view = state.shared.projection.read().graph(&id);
    view.map(|v| Json(v).into_response())
        .ok_or_else(|| AppError::NotFound(format!("graph {id}")))
}

#[derive(Deserialize)]
struct TaskQuery {
    depth: Option<usize>,
}

async fn task(
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
    Query(query): Query<TaskQuery>,
) -> Result<Response, AppError> {
    let depth = query.depth.unwrap_or(6).min(32);
    let view = state.shared.projection.read().task(&id, &name, depth);
    view.map(|v| Json(v).into_response())
        .ok_or_else(|| AppError::NotFound(format!("task {name} in graph {id}")))
}

#[derive(Deserialize)]
struct RunsQuery {
    graph: Option<String>,
    limit: Option<usize>,
}

async fn runs(State(state): State<AppState>, Query(query): Query<RunsQuery>) -> Response {
    let limit = query.limit.unwrap_or(50).min(1_000);
    Json(
        state
            .shared
            .projection
            .read()
            .runs(query.graph.as_deref(), limit),
    )
    .into_response()
}

async fn run(State(state): State<AppState>, UuidPath(id): UuidPath) -> Result<Response, AppError> {
    let projection = state.shared.projection.read();
    projection
        .run(id)
        .map(|r| Json(r).into_response())
        .ok_or_else(|| AppError::NotFound(format!("run {id}")))
}

/// Live facts as named SSE events (`task_finished`, `run_started`, …) whose
/// data is the JSON `TaskgraphEvent`. Live only: a client (re)fetches the
/// views on connect and applies events on top.
async fn sse(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let connected = futures::stream::once(async { Ok(Event::default().comment("connected")) });
    let events =
        BroadcastStream::new(state.shared.events.subscribe()).filter_map(|item| async move {
            match item {
                Ok(event) => match Event::default().event(event.body.kind()).json_data(&event) {
                    Ok(sse) => Some(Ok(sse)),
                    Err(e) => {
                        warn!(error = %e, "failed to serialize TASKGRAPH event for SSE");
                        None
                    }
                },
                Err(BroadcastStreamRecvError::Lagged(skipped)) => Some(Ok(Event::default()
                    .event("lagged")
                    .data(skipped.to_string()))),
            }
        });
    Sse::new(connected.chain(events)).keep_alive(KeepAlive::default())
}
