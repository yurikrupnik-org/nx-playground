use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Sse, sse},
    routing::get,
};
use futures::Stream;
use k8s_crd_groupers::{DashboardData, LiveState};
use kube::Client;
use serde::Deserialize;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::EnvFilter;

struct AppState {
    client: Client,
    live: Arc<LiveState>,
}

#[derive(Deserialize)]
struct ResourcePath {
    group: String,
    version: String,
    kind: String,
    name: String,
}

#[derive(Deserialize)]
struct NsQuery {
    #[serde(default)]
    namespace: Option<String>,
}

#[derive(Deserialize)]
struct YamlBody {
    yaml: String,
}

/// GET /api/dashboard - current live state
async fn get_dashboard(State(state): State<Arc<AppState>>) -> Json<DashboardData> {
    Json(state.live.get_dashboard().await)
}

/// GET /api/events - SSE stream of dashboard updates
async fn sse_events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    let mut rx = state.live.subscribe();

    // Send initial state immediately
    let initial_data = state.live.get_dashboard().await;
    let initial_json = serde_json::to_string(&initial_data).unwrap_or_default();

    let stream = async_stream::stream! {
        // Send initial snapshot
        yield Ok(sse::Event::default().event("snapshot").data(initial_json));

        // Then send updates whenever state changes
        loop {
            match rx.recv().await {
                Ok(()) => {
                    let data = state.live.get_dashboard().await;
                    match serde_json::to_string(&data) {
                        Ok(json) => {
                            yield Ok(sse::Event::default().event("update").data(json));
                        }
                        Err(e) => {
                            tracing::error!("Failed to serialize dashboard: {}", e);
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("SSE client lagged by {} messages, sending latest", n);
                    let data = state.live.get_dashboard().await;
                    if let Ok(json) = serde_json::to_string(&data) {
                        yield Ok(sse::Event::default().event("update").data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(stream).keep_alive(
        sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

/// GET /api/resource/:group/:version/:kind/:name?namespace=...
async fn get_resource(
    State(state): State<Arc<AppState>>,
    Path(path): Path<ResourcePath>,
    Query(ns): Query<NsQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let yaml = k8s_crd_groupers::get_resource_yaml(
        &state.client,
        &path.group,
        &path.version,
        &path.kind,
        ns.namespace.as_deref(),
        &path.name,
    )
    .await
    .map_err(|e| (StatusCode::NOT_FOUND, format!("Resource not found: {e}")))?;

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "text/yaml".parse().unwrap());
    Ok((headers, yaml))
}

/// PUT /api/resource/:group/:version/:kind/:name?namespace=...
async fn update_resource(
    State(state): State<Arc<AppState>>,
    Path(path): Path<ResourcePath>,
    Query(ns): Query<NsQuery>,
    Json(body): Json<YamlBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let yaml = k8s_crd_groupers::apply_resource_yaml(
        &state.client,
        &path.group,
        &path.version,
        &path.kind,
        ns.namespace.as_deref(),
        &path.name,
        &body.yaml,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Failed to apply: {e}"),
        )
    })?;

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "text/yaml".parse().unwrap());
    Ok((headers, yaml))
}

/// GET / - serve the dashboard
async fn dashboard() -> Html<&'static str> {
    Html(include_str!("../../static/index.html"))
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let client = k8s_crd_groupers::create_client().await?;

    // Create live state and spawn background watchers
    let (live, _rx) = LiveState::new();
    k8s_crd_groupers::spawn_watchers(client.clone(), live.clone());

    let state = Arc::new(AppState { client, live });

    let app = Router::new()
        .route("/", get(dashboard))
        .route("/api/dashboard", get(get_dashboard))
        .route("/api/events", get(sse_events))
        .route(
            "/api/resource/{group}/{version}/{kind}/{name}",
            get(get_resource).put(update_resource),
        )
        .route("/health", get(health))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("CRD Dashboard listening on http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
