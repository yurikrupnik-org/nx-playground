//! The local web servers behind `butler submodule serve` and `butler ui serve`.
//!
//! Single-user and local by design: every state-changing request is serialised
//! behind one lock, and each page is compiled into the binary, so there is
//! nothing to deploy.
//!
//! Cross-site safety for a server that edits the repo: every state change takes
//! a JSON body or a non-simple method, so a foreign page cannot send one without
//! a CORS preflight, which these servers never answer. Bound to loopback, they
//! also reject any `Host` that is not a loopback name, which closes DNS
//! rebinding.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub struct AppState {
    /// What every request works on: the repo root (`submodule serve`) or the
    /// butler.toml (`ui serve`).
    path: PathBuf,
    loopback: bool,
    lock: Mutex<()>,
}

pub type Shared = Arc<AppState>;

/// Bind `addr` and serve `app` until Ctrl-C. `what` names the page in the
/// startup line.
pub fn run(path: PathBuf, addr: SocketAddr, what: &str, app: Router<Shared>) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let state = Arc::new(AppState {
            path,
            loopback: addr.ip().is_loopback(),
            lock: Mutex::new(()),
        });
        let app = app
            .layer(middleware::from_fn_with_state(state.clone(), guard_host))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .wrap_err_with(|| format!("binding {addr}"))?;
        println!(
            "butler {what} on http://{} (Ctrl-C to stop)",
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
        if !is_loopback_name(host_name(host)) {
            return (StatusCode::FORBIDDEN, "unexpected Host header").into_response();
        }
    }
    next.run(req).await
}

/// A host name that always resolves to this machine.
pub fn is_loopback_name(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// `localhost:7878` -> `localhost`, `[::1]:7878` -> `[::1]`.
pub fn host_name(host: &str) -> &str {
    if host.starts_with('[') {
        host.find(']').map_or(host, |end| &host[..=end])
    } else {
        host.split(':').next().unwrap_or(host)
    }
}

pub struct ApiError(pub eyre::Report);

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

pub type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

/// Run blocking repo work off the async workers, one request at a time.
pub async fn locked<T, F>(state: &Shared, work: F) -> ApiResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Path) -> Result<T> + Send + 'static,
{
    let _guard = state.lock.lock().await;
    let path = state.path.clone();
    tokio::task::spawn_blocking(move || work(&path))
        .await
        .map_err(|e| ApiError(eyre!("worker failed: {e}")))?
        .map(Json)
        .map_err(ApiError)
}

/// An empty JSON object. A change that needs no input still takes one, so it
/// cannot be triggered by a cross-site form post.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

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
