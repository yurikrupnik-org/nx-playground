//! HTTP → gRPC handlers for the tasks service.
//!
//! This is the *caller's* code and lives in the caller. It depends on the published
//! contract (`contract_tasks`) and the generated wire types (`rpc::tasks::v1`) - never
//! on `domain_tasks`, which holds the service's storage and business logic. See
//! `docs/adr-tasks-service-boundary.md`.

use std::time::Duration;

use axum::{
    Extension, Json, Router, extract::Query, extract::State, http::StatusCode,
    response::IntoResponse, routing::get,
};
use axum_helpers::UuidPath;
use grpc_client::TracedChannel;
use oidc_auth::AccessToken;
use rpc::tasks::v1::tasks_service_client::TasksServiceClient;
use rpc::tasks::v1::{DeleteByIdRequest, GetByIdRequest, ListRequest};
use utoipa::OpenApi;

use contract_tasks::conversions::*;
use contract_tasks::{CreateTask, Task, TaskFilter, UpdateTask};

use crate::error::ApiResult;

/// Per-call deadline for the tasks service.
///
/// The channel's connect/request timeouts are a backstop for a *dead* peer; this bounds
/// a peer that is merely slow, so one degraded downstream cannot pin BFF request slots.
const TASKS_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Wrap a message as an authenticated, deadline-bounded gRPC request.
///
/// The caller's own access token is forwarded verbatim; the tasks service verifies it
/// against the same JWKS and derives the tenant itself. Identity is therefore never a
/// field we fill in - see `docs/adr-tasks-service-boundary.md` Phase 4.
fn authed<T>(token: &AccessToken, message: T) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    // Infallible: a JWT is ASCII, and the value came from a verified token.
    if let Ok(value) = format!("Bearer {}", token.0).parse() {
        request.metadata_mut().insert("authorization", value);
    }
    request.set_timeout(TASKS_CALL_TIMEOUT);
    request
}

/// OpenAPI documentation for the tasks routes.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Tasks API",
        version = "1.0.0",
        description = "REST facade over the tasks gRPC service (rpc tasks.v1)"
    ),
    servers((url = "/api/tasks", description = "zerg-api mount path")),
    paths(list_tasks, get_task, create_task, update_task, delete_task,),
    components(schemas(Task, CreateTask, UpdateTask)),
    tags((name = "tasks", description = "Task operations (backed by the tasks gRPC service)"))
)]
pub struct TasksApiDoc;

pub fn router(state: crate::state::AppState) -> Router {
    Router::new()
        .route("/", get(list_tasks).post(create_task))
        .route("/{id}", get(get_task).put(update_task).delete(delete_task))
        .with_state(state.tasks_client.clone())
}

/// List tasks
#[utoipa::path(
    get,
    path = "",
    tag = "tasks",
    params(TaskFilter),
    responses(
        (status = 200, description = "List of tasks", body = Vec<Task>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_tasks(
    State(mut client): State<TasksServiceClient<TracedChannel>>,
    Extension(token): Extension<AccessToken>,
    Query(filter): Query<TaskFilter>,
) -> ApiResult<Json<Vec<Task>>> {
    let response = client
        .list(authed(
            &token,
            ListRequest {
                mine: filter.mine,
                project_id: opt_uuid_to_bytes(filter.project_id),
                status: filter.status.map(Into::into),
                priority: filter.priority.map(Into::into),
                completed: filter.completed,
                limit: filter.limit as i32,
                offset: filter.offset as i32,
            },
        ))
        .await?;

    Ok(Json(list_response_to_tasks(response.into_inner())?))
}

/// Get a task by ID
#[utoipa::path(
    get,
    path = "/{id}",
    tag = "tasks",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    responses(
        (status = 200, description = "Task found", body = Task),
        (status = 400, description = "Invalid task ID"),
        (status = 404, description = "Task not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_task(
    State(mut client): State<TasksServiceClient<TracedChannel>>,
    Extension(token): Extension<AccessToken>,
    UuidPath(uuid): UuidPath,
) -> ApiResult<impl IntoResponse> {
    let response = client
        .get_by_id(authed(
            &token,
            GetByIdRequest {
                id: uuid_to_bytes(uuid),
            },
        ))
        .await?;

    let task: Task = response.into_inner().try_into()?;

    Ok(Json(task))
}

/// Create a new task
#[utoipa::path(
    post,
    path = "",
    tag = "tasks",
    request_body = CreateTask,
    responses(
        (status = 201, description = "Task created successfully", body = Task),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn create_task(
    State(mut client): State<TasksServiceClient<TracedChannel>>,
    Extension(token): Extension<AccessToken>,
    Json(input): Json<CreateTask>,
) -> ApiResult<impl IntoResponse> {
    let response = client.create(authed(&token, input.into())).await?;

    let task: Task = response.into_inner().try_into()?;

    Ok((StatusCode::CREATED, Json(task)))
}

/// Update a task
#[utoipa::path(
    put,
    path = "/{id}",
    tag = "tasks",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    request_body = UpdateTask,
    responses(
        (status = 200, description = "Task updated successfully", body = Task),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Task not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_task(
    State(mut client): State<TasksServiceClient<TracedChannel>>,
    Extension(token): Extension<AccessToken>,
    UuidPath(uuid): UuidPath,
    Json(input): Json<UpdateTask>,
) -> ApiResult<impl IntoResponse> {
    let response = client
        .update_by_id(authed(&token, make_update_request(uuid, input)))
        .await?;

    let task: Task = response.into_inner().try_into()?;

    Ok(Json(task))
}

/// Delete a task
#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "tasks",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    responses(
        (status = 204, description = "Task deleted successfully"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Task not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_task(
    State(mut client): State<TasksServiceClient<TracedChannel>>,
    Extension(token): Extension<AccessToken>,
    UuidPath(uuid): UuidPath,
) -> ApiResult<impl IntoResponse> {
    client
        .delete_by_id(authed(
            &token,
            DeleteByIdRequest {
                id: uuid_to_bytes(uuid),
            },
        ))
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use utoipa::OpenApi;

    /// Regenerates the committed OpenAPI v1 document. Same convention as the
    /// ts-rs `export_bindings_*` tests: running the suite keeps
    /// `docs/openapi/tasks.v1.json` in sync with the handler annotations.
    #[test]
    fn export_openapi_tasks_v1() {
        let json = super::TasksApiDoc::openapi()
            .to_pretty_json()
            .expect("serialize tasks openapi doc");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../docs/openapi/tasks.v1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, json + "\n").unwrap();
    }
}
