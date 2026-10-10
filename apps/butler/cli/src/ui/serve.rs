//! `butler ui serve` — one page that frames every registered UI, edits the
//! registry, and lists the ports in use. Server, lock and cross-site guards:
//! [`crate::web`].

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::{Path as UrlPath, State};
use axum::response::Html;
use axum::routing::{get, patch};
use axum::{Json, Router};
use eyre::{Result, eyre};

use super::ports::{self, UsedPort};
use super::{AddRequest, Probe, SetRequest, Ui};
use crate::web::{ApiError, ApiResult, Shared, locked};

const PAGE: &str = include_str!("index.html");

pub fn run(file: PathBuf, addr: SocketAddr) -> Result<()> {
    let app = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/api/uis", get(list).post(add))
        .route("/api/uis/{name}", patch(set).delete(remove))
        .route("/api/uis/{name}/probe", get(probe))
        .route("/api/ports", get(used_ports));
    crate::web::run(file, addr, "UI dashboard", app)
}

async fn list(State(state): State<Shared>) -> ApiResult<Vec<Ui>> {
    locked(&state, super::load).await
}

/// Every change answers with the new list.
async fn add(State(state): State<Shared>, Json(req): Json<AddRequest>) -> ApiResult<Vec<Ui>> {
    locked(&state, move |file| {
        super::add(file, &req)?;
        super::load(file)
    })
    .await
}

async fn set(
    State(state): State<Shared>,
    UrlPath(name): UrlPath<String>,
    Json(req): Json<SetRequest>,
) -> ApiResult<Vec<Ui>> {
    locked(&state, move |file| {
        super::set(file, &name, &req)?;
        super::load(file)
    })
    .await
}

async fn remove(State(state): State<Shared>, UrlPath(name): UrlPath<String>) -> ApiResult<Vec<Ui>> {
    locked(&state, move |file| {
        super::remove(file, &name)?;
        super::load(file)
    })
    .await
}

/// Probes run outside the lock: they wait on the network, not on the repo,
/// and the page fires one per UI at once.
async fn probe(State(state): State<Shared>, UrlPath(name): UrlPath<String>) -> ApiResult<Probe> {
    let Json(ui) = locked(&state, move |file| {
        super::load(file)?
            .into_iter()
            .find(|ui| ui.name == name)
            .ok_or_else(|| eyre!("no UI named `{name}`"))
    })
    .await?;
    tokio::task::spawn_blocking(move || super::probe(&ui))
        .await
        .map(Json)
        .map_err(|e| ApiError(eyre!("probe failed: {e}")))
}

async fn used_ports(State(state): State<Shared>) -> ApiResult<Vec<UsedPort>> {
    locked(&state, |file| ports::used(&super::load(file)?)).await
}
