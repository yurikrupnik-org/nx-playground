//! `butler submodule serve` — a local web UI over the same functions the CLI
//! calls.
//!
//! Single-user and local by design: every request is serialised behind one
//! lock (git's index lock would reject concurrent changes anyway), and the
//! page is compiled into the binary, so there is nothing to deploy.
//!
//! Cross-site safety for a server that runs git: every state change takes a
//! JSON body or a non-simple method, so a foreign page cannot send one without
//! a CORS preflight, which this server never answers. Bound to loopback, it
//! also rejects any `Host` that is not a loopback name, which closes DNS
//! rebinding.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as UrlPath, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::manifests::{self, GenReport};
use super::{AddRequest, SetRequest, Submodule, UpdateRequest};

const PAGE: &str = include_str!("index.html");

struct AppState {
    root: PathBuf,
    loopback: bool,
    lock: Mutex<()>,
}

type Shared = Arc<AppState>;

pub fn run(root: PathBuf, addr: SocketAddr) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let state = Arc::new(AppState {
            root,
            loopback: addr.ip().is_loopback(),
            lock: Mutex::new(()),
        });
        let app = Router::new()
            .route("/", get(|| async { Html(PAGE) }))
            .route("/api/submodules", get(list).post(add))
            .route("/api/submodules/{*name}", patch(set).delete(remove))
            .route("/api/update", post(update))
            .route("/api/manifests", get(preview).post(generate))
            .layer(middleware::from_fn_with_state(state.clone(), guard_host))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .wrap_err_with(|| format!("binding {addr}"))?;
        println!(
            "butler submodule UI on http://{} (Ctrl-C to stop)",
            listener.local_addr()?
        );
        axum::serve(listener, app).await?;
        Ok(())
    })
}

async fn guard_host(State(state): State<Shared>, req: Request, next: Next) -> Response {
    if state.loopback {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();
        if !matches!(host_name(host), "localhost" | "127.0.0.1" | "[::1]") {
            return (StatusCode::FORBIDDEN, "unexpected Host header").into_response();
        }
    }
    next.run(req).await
}

/// `localhost:7878` -> `localhost`, `[::1]:7878` -> `[::1]`.
fn host_name(host: &str) -> &str {
    if host.starts_with('[') {
        host.find(']').map_or(host, |end| &host[..=end])
    } else {
        host.split(':').next().unwrap_or(host)
    }
}

// ---------------------------------------------------------------------------
// Handlers

struct ApiError(eyre::Report);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body {
            error: String,
        }
        let body = Body {
            error: format!("{:#}", self.0),
        };
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}

type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

/// Run blocking git/kcl work off the async workers, one request at a time.
async fn locked<T, F>(state: &Shared, work: F) -> ApiResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Path) -> Result<T> + Send + 'static,
{
    let _guard = state.lock.lock().await;
    let root = state.root.clone();
    tokio::task::spawn_blocking(move || work(&root))
        .await
        .map_err(|e| ApiError(eyre!("worker failed: {e}")))?
        .map(Json)
        .map_err(ApiError)
}

/// What every change returns: the new list, and what regeneration did.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Outcome {
    submodules: Vec<Submodule>,
    generated: Option<GenReport>,
}

fn outcome(root: &Path) -> Result<Outcome> {
    let settings = super::load_settings(root)?;
    let generated = super::regenerate_after_change(root, settings.as_ref())?;
    Ok(Outcome {
        submodules: super::load(root)?,
        generated,
    })
}

async fn list(State(state): State<Shared>) -> ApiResult<Vec<Submodule>> {
    locked(&state, super::load).await
}

async fn add(State(state): State<Shared>, Json(req): Json<AddRequest>) -> ApiResult<Outcome> {
    locked(&state, move |root| {
        super::add(root, &req)?;
        outcome(root)
    })
    .await
}

async fn set(
    State(state): State<Shared>,
    UrlPath(name): UrlPath<String>,
    Json(req): Json<SetRequest>,
) -> ApiResult<Outcome> {
    locked(&state, move |root| {
        super::set(root, name.trim_start_matches('/'), &req)?;
        outcome(root)
    })
    .await
}

async fn remove(State(state): State<Shared>, UrlPath(name): UrlPath<String>) -> ApiResult<Outcome> {
    locked(&state, move |root| {
        let settings = super::load_settings(root)?;
        super::remove(root, settings.as_ref(), name.trim_start_matches('/'))?;
        outcome(root)
    })
    .await
}

async fn update(State(state): State<Shared>, Json(req): Json<UpdateRequest>) -> ApiResult<Outcome> {
    locked(&state, move |root| {
        super::update(root, &req)?;
        outcome(root)
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Preview {
    out_dir: String,
    files: Vec<PreviewFile>,
    stale: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewFile {
    path: String,
    /// `unchanged`, `changed` or `new` relative to the file on disk.
    status: &'static str,
    content: String,
}

async fn preview(State(state): State<Shared>) -> ApiResult<Preview> {
    locked(&state, |root| {
        let plan = plan(root)?;
        let files = plan
            .files
            .into_iter()
            .map(|(path, content)| {
                let status = match std::fs::read_to_string(root.join(&path)) {
                    Ok(on_disk) if on_disk == content => "unchanged",
                    Ok(_) => "changed",
                    Err(_) => "new",
                };
                PreviewFile {
                    path,
                    status,
                    content,
                }
            })
            .collect();
        Ok(Preview {
            out_dir: plan.out_dir,
            files,
            stale: plan.stale,
        })
    })
    .await
}

/// An empty JSON object. Required so that regeneration, like every other
/// change, cannot be triggered by a cross-site form post.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

async fn generate(State(state): State<Shared>, Json(_): Json<Empty>) -> ApiResult<GenReport> {
    locked(&state, |root| plan(root)?.apply(root)).await
}

fn plan(root: &Path) -> Result<manifests::Plan> {
    let settings = super::load_settings(root)?.ok_or_else(|| {
        eyre!(
            "manifest generation needs a {} at the workspace root",
            crate::settings::FILE
        )
    })?;
    manifests::plan(root, &settings, &super::load(root)?)
}

#[cfg(test)]
mod tests {
    use super::host_name;

    #[test]
    fn host_name_strips_the_port_but_keeps_ipv6_brackets() {
        assert_eq!(host_name("localhost:7878"), "localhost");
        assert_eq!(host_name("[::1]:7878"), "[::1]");
        assert_eq!(host_name("evil.example"), "evil.example");
    }
}
