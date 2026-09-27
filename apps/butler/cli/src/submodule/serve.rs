//! `butler submodule serve` — a local web UI over the same functions the CLI
//! calls. Server, lock and cross-site guards: [`crate::web`].

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum::extract::{Path as UrlPath, State};
use axum::response::Html;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use eyre::{Result, eyre};
use serde::Serialize;

use super::manifests::{self, GenReport};
use super::{AddRequest, SetRequest, Submodule, UpdateRequest};
use crate::web::{ApiResult, Empty, Shared, locked};

const PAGE: &str = include_str!("index.html");

pub fn run(root: PathBuf, addr: SocketAddr) -> Result<()> {
    let app = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/api/submodules", get(list).post(add))
        .route("/api/submodules/{*name}", patch(set).delete(remove))
        .route("/api/update", post(update))
        .route("/api/manifests", get(preview).post(generate));
    crate::web::run(root, addr, "submodule UI", app)
}

// ---------------------------------------------------------------------------
// Handlers

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
