//! Walks the axum-helpers HTTP building blocks — [`create_router`],
//! [`health_router`], [`ValidatedJson`], [`UuidPath`], and [`AppError`] — over a
//! real socket, then exits. This is the crate's counterpart to the persistence
//! example in `libs/domains/todo/examples/` and the event example in
//! `apps/zerg/email-nats/examples/`.
//!
//! Run:
//!
//! ```sh
//! CORS_ALLOWED_ORIGIN=http://localhost:3000 \
//!   cargo run -p axum-helpers --example http_service
//! ```
//!
//! `tests/examples_it.rs` runs this exact command as a process, so the
//! walkthrough cannot rot without a test going red. No Docker required — the
//! server binds an ephemeral localhost port and the example talks to itself.
//!
//! There is no store behind `/api/messages/{id}`: every well-formed id is a
//! miss. The point is the structured error envelope each failure mode produces,
//! not persistence — that side lives in the todo example.

use axum::routing::{get, post};
use axum::{Json, Router, http::StatusCode};
use axum_helpers::errors::AppError;
use axum_helpers::extractors::{UuidPath, ValidatedJson};
use axum_helpers::server::{create_router, health_router};
use core_config::app_info;
use serde::{Deserialize, Serialize};
use utoipa::OpenApi;
use uuid::Uuid;
use validator::Validate;

#[derive(OpenApi)]
#[openapi(paths())]
struct ApiDoc;

#[derive(Deserialize, Validate)]
struct CreateMessage {
    #[validate(length(min = 1, max = 80))]
    text: String,
}

#[derive(Serialize)]
struct Message {
    id: Uuid,
    text: String,
}

/// `ValidatedJson` rejects an out-of-bounds `text` with a 400 and a
/// `VALIDATION_ERROR` envelope before this body ever runs.
async fn create_message(
    ValidatedJson(body): ValidatedJson<CreateMessage>,
) -> (StatusCode, Json<Message>) {
    (
        StatusCode::CREATED,
        Json(Message {
            id: Uuid::new_v4(),
            text: body.text,
        }),
    )
}

/// `UuidPath` turns a malformed id into a 400 before this body runs; a
/// well-formed one falls through to the structured 404 below.
async fn get_message(UuidPath(id): UuidPath) -> Result<Json<Message>, AppError> {
    Err(AppError::NotFound(format!("message {id} not found")))
}

/// Lines prefixed `step:` are asserted on by `tests/examples_it.rs` — keep them
/// and the test in sync.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ---- assemble ----------------------------------------------------------
    // create_router nests these under /api and layers CORS (from
    // CORS_ALLOWED_ORIGIN — required, startup fails without it), security
    // headers, tracing, compression, the OpenAPI docs UIs, and the 404 fallback.
    let api = Router::new()
        .route("/messages", post(create_message))
        .route("/messages/{id}", get(get_message));

    let router = create_router::<ApiDoc>(api)
        .await?
        .merge(health_router(app_info!()));

    // Port 0: the OS picks a free port, so the example never collides with a
    // dev server. create_app/create_production_app are the run-forever entry
    // points; this walkthrough serves, proves behavior, and exits.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    println!("step:listening addr={addr}");
    let server = tokio::spawn(async move { axum::serve(listener, router).await });

    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // ---- liveness -----------------------------------------------------------
    // health_router is merged at the top level, not under /api.
    let resp = client.get(format!("{base}/health")).send().await?;
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await?;
    println!(
        "step:health status={status} name={}",
        body["name"].as_str().unwrap_or("?")
    );

    // ---- happy path ---------------------------------------------------------
    let resp = client
        .post(format!("{base}/api/messages"))
        .json(&serde_json::json!({ "text": "hello" }))
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await?;
    println!(
        "step:created status={status} id={}",
        body["id"].as_str().unwrap_or("?")
    );
    if status != 201 {
        return Err(format!("expected 201 from valid create, got {status}").into());
    }

    // ---- validation failure -------------------------------------------------
    // Empty text violates length(min = 1); ValidatedJson answers with the
    // ErrorResponse envelope and per-field details.
    let resp = client
        .post(format!("{base}/api/messages"))
        .json(&serde_json::json!({ "text": "" }))
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await?;
    println!(
        "step:rejected status={status} error={}",
        body["error"].as_str().unwrap_or("?")
    );

    // ---- malformed path parameter --------------------------------------------
    let resp = client
        .get(format!("{base}/api/messages/not-a-uuid"))
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await?;
    println!(
        "step:bad_uuid status={status} error={}",
        body["error"].as_str().unwrap_or("?")
    );

    // ---- structured domain 404 -----------------------------------------------
    let resp = client
        .get(format!("{base}/api/messages/{}", Uuid::new_v4()))
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await?;
    println!(
        "step:not_found status={status} error={}",
        body["error"].as_str().unwrap_or("?")
    );

    // ---- fallback 404 ----------------------------------------------------------
    // Unrouted paths get the same envelope from create_router's fallback, not a
    // bare axum default.
    let resp = client.get(format!("{base}/no-such-route")).send().await?;
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await?;
    println!(
        "step:fallback status={status} error={}",
        body["error"].as_str().unwrap_or("?")
    );

    server.abort();
    println!("HTTP walkthrough complete");
    Ok(())
}
