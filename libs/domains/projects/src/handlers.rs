use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use axum_helpers::{
    AuditEvent, AuditOutcome, UuidPath,
    errors::responses::{
        BadRequestUuidResponse, BadRequestValidationResponse, ConflictResponse,
        InternalServerErrorResponse, NotFoundResponse,
    },
    extract_ip_from_headers, extract_user_agent,
};
use core_proc_macros::ApiResource;
use serde_json::json;
use std::sync::Arc;
use utoipa::OpenApi;

use crate::entity;
use crate::error::ProjectResult;
use crate::models::{CreateProject, Project, ProjectFilter, UpdateProject};
use crate::repository::ProjectRepository;
use crate::service::ProjectService;

/// OpenAPI documentation for Projects API
#[derive(OpenApi)]
#[openapi(
    paths(
        list_projects,
        create_project,
        get_project,
        update_project,
        delete_project,
        activate_project,
        suspend_project,
        archive_project,
    ),
    components(
        schemas(Project, CreateProject, UpdateProject, ProjectFilter),
        responses(
            NotFoundResponse,
            BadRequestValidationResponse,
            BadRequestUuidResponse,
            ConflictResponse,
            InternalServerErrorResponse
        )
    ),
    tags(
        (name = entity::Model::TAG, description = "Project management endpoints")
    )
)]
pub struct ApiDoc;

/// Create the project router with all HTTP endpoints
pub fn router<R: ProjectRepository + 'static>(service: ProjectService<R>) -> Router {
    let shared_service = Arc::new(service);

    Router::new()
        .route("/", get(list_projects).post(create_project))
        .route(
            "/{id}",
            get(get_project).put(update_project).delete(delete_project),
        )
        .route("/{id}/activate", post(activate_project))
        .route("/{id}/suspend", post(suspend_project))
        .route("/{id}/archive", post(archive_project))
        .with_state(shared_service)
}

/// List projects with optional filters
#[utoipa::path(
    get,
    path = "",
    tag = entity::Model::TAG,
    params(ProjectFilter),
    responses(
        (status = 200, description = "List of projects", body = Vec<Project>),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn list_projects<R: ProjectRepository>(
    State(service): State<Arc<ProjectService<R>>>,
    Query(filter): Query<ProjectFilter>,
) -> ProjectResult<Json<Vec<Project>>> {
    let projects = service.list_projects(filter).await?;
    Ok(Json(projects))
}

/// Create a new project
#[utoipa::path(
    post,
    path = "",
    tag = entity::Model::TAG,
    request_body = CreateProject,
    responses(
        (status = 201, description = "Project created successfully", body = Project),
        (status = 400, response = BadRequestValidationResponse),
        (status = 409, response = ConflictResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn create_project<R: ProjectRepository>(
    State(service): State<Arc<ProjectService<R>>>,
    headers: HeaderMap,
    Json(input): Json<CreateProject>,
) -> ProjectResult<impl IntoResponse> {
    let project = service.create_project(input).await?;

    // Audit log successful creation
    AuditEvent::new(
        Some(project.user_id.to_string()),
        "project.create",
        Some(format!("project:{}", project.id)),
        AuditOutcome::Success,
    )
    .with_ip(extract_ip_from_headers(&headers))
    .with_user_agent(extract_user_agent(&headers))
    .with_details(json!({
        "project_name": project.name,
        "cloud_provider": project.cloud_provider.to_string(),
        "region": project.region,
        "environment": project.environment.to_string(),
    }))
    .log();

    Ok((StatusCode::CREATED, Json(project)))
}

/// Get a project by ID
#[utoipa::path(
    get,
    path = "/{id}",
    tag = entity::Model::TAG,
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    responses(
        (status = 200, description = "Project found", body = Project),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn get_project<R: ProjectRepository>(
    State(service): State<Arc<ProjectService<R>>>,
    UuidPath(id): UuidPath,
) -> ProjectResult<Json<Project>> {
    let project = service.get_project(id).await?;
    Ok(Json(project))
}

/// Update a project
#[utoipa::path(
    put,
    path = "/{id}",
    tag = entity::Model::TAG,
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    request_body = UpdateProject,
    responses(
        (status = 200, description = "Project updated successfully", body = Project),
        (status = 400, response = BadRequestValidationResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn update_project<R: ProjectRepository>(
    State(service): State<Arc<ProjectService<R>>>,
    UuidPath(id): UuidPath,
    Json(input): Json<UpdateProject>,
) -> ProjectResult<Json<Project>> {
    let project = service.update_project(id, input).await?;
    Ok(Json(project))
}

/// Delete a project
#[utoipa::path(
    delete,
    path = "/{id}",
    tag = entity::Model::TAG,
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    responses(
        (status = 204, description = "Project deleted successfully"),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn delete_project<R: ProjectRepository>(
    State(service): State<Arc<ProjectService<R>>>,
    headers: HeaderMap,
    UuidPath(id): UuidPath,
) -> ProjectResult<impl IntoResponse> {
    service.delete_project(id).await?;

    // Audit log successful deletion
    AuditEvent::new(
        None, // TODO: Add user_id when authentication is implemented
        "project.delete",
        Some(format!("project:{}", id)),
        AuditOutcome::Success,
    )
    .with_ip(extract_ip_from_headers(&headers))
    .with_user_agent(extract_user_agent(&headers))
    .log();

    Ok(StatusCode::NO_CONTENT)
}

/// Activate a project
#[utoipa::path(
    post,
    path = "/{id}/activate",
    tag = entity::Model::TAG,
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    responses(
        (status = 200, description = "Project activated successfully", body = Project),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn activate_project<R: ProjectRepository>(
    State(service): State<Arc<ProjectService<R>>>,
    UuidPath(id): UuidPath,
) -> ProjectResult<Json<Project>> {
    let project = service.activate_project(id).await?;
    Ok(Json(project))
}

/// Suspend a project
#[utoipa::path(
    post,
    path = "/{id}/suspend",
    tag = entity::Model::TAG,
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    responses(
        (status = 200, description = "Project suspended successfully", body = Project),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn suspend_project<R: ProjectRepository>(
    State(service): State<Arc<ProjectService<R>>>,
    UuidPath(id): UuidPath,
) -> ProjectResult<Json<Project>> {
    let project = service.suspend_project(id).await?;
    Ok(Json(project))
}

/// Archive a project
#[utoipa::path(
    post,
    path = "/{id}/archive",
    tag = entity::Model::TAG,
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    responses(
        (status = 200, description = "Project archived successfully", body = Project),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn archive_project<R: ProjectRepository>(
    State(service): State<Arc<ProjectService<R>>>,
    UuidPath(id): UuidPath,
) -> ProjectResult<Json<Project>> {
    let project = service.archive_project(id).await?;
    Ok(Json(project))
}
