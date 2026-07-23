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
        BadRequestUuidResponse, BadRequestValidationResponse, InternalServerErrorResponse,
        NotFoundResponse,
    },
    extract_ip_from_headers, extract_user_agent,
};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use utoipa::{OpenApi, ToSchema};

use crate::{
    error::CloudResourceResult,
    models::{CloudResource, CloudResourceFilter, CreateCloudResource, UpdateCloudResource},
    repository::CloudResourceRepository,
    service::CloudResourceService,
};

/// OpenAPI documentation for Cloud Resources API
#[derive(OpenApi)]
#[openapi(
    paths(
        create_cloud_resource,
        get_cloud_resource,
        list_cloud_resources,
        list_by_project,
        update_cloud_resource,
        delete_cloud_resource,
        soft_delete_cloud_resource,
    ),
    components(
        schemas(
            CloudResource,
            CreateCloudResource,
            UpdateCloudResource,
            CloudResourceFilter,
            MessageResponse
        ),
        responses(
            NotFoundResponse,
            BadRequestValidationResponse,
            BadRequestUuidResponse,
            InternalServerErrorResponse
        )
    ),
    tags(
        (name = "cloud-resources", description = "Cloud resource management endpoints")
    )
)]
pub struct ApiDoc;

/// Message response for delete operations
#[derive(Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}

/// Create Axum router for cloud resource endpoints
pub fn router<R>(service: CloudResourceService<R>) -> Router
where
    R: CloudResourceRepository + 'static,
{
    let service = Arc::new(service);

    Router::new()
        .route("/", post(create_cloud_resource).get(list_cloud_resources))
        .route(
            "/{id}",
            get(get_cloud_resource)
                .put(update_cloud_resource)
                .delete(delete_cloud_resource),
        )
        .route("/project/{project_id}", get(list_by_project))
        .route("/{id}/soft-delete", post(soft_delete_cloud_resource))
        .with_state(service)
}

/// Create a new cloud resource
#[utoipa::path(
    post,
    path = "",
    tag = "cloud-resources",
    request_body = CreateCloudResource,
    responses(
        (status = 201, description = "Cloud resource created successfully", body = CloudResource),
        (status = 400, response = BadRequestValidationResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn create_cloud_resource<R>(
    State(service): State<Arc<CloudResourceService<R>>>,
    headers: HeaderMap,
    Json(input): Json<CreateCloudResource>,
) -> CloudResourceResult<impl IntoResponse>
where
    R: CloudResourceRepository,
{
    let resource = service.create(input).await?;

    // Audit log successful creation
    AuditEvent::new(
        None, // TODO: Add user_id when authentication is implemented
        "cloud_resource.create",
        Some(format!("cloud_resource:{}", resource.id)),
        AuditOutcome::Success,
    )
    .with_ip(extract_ip_from_headers(&headers))
    .with_user_agent(extract_user_agent(&headers))
    .with_details(json!({
        "resource_type": resource.resource_type,
        "name": resource.name,
        "region": resource.region,
        "project_id": resource.project_id,
    }))
    .log();

    Ok((StatusCode::CREATED, Json(resource)))
}

/// Get a cloud resource by ID
#[utoipa::path(
    get,
    path = "/{id}",
    tag = "cloud-resources",
    params(
        ("id" = Uuid, Path, description = "Cloud resource ID")
    ),
    responses(
        (status = 200, description = "Cloud resource found", body = CloudResource),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn get_cloud_resource<R>(
    State(service): State<Arc<CloudResourceService<R>>>,
    UuidPath(id): UuidPath,
) -> CloudResourceResult<impl IntoResponse>
where
    R: CloudResourceRepository,
{
    let resource = service.get(id).await?;
    Ok(Json(resource))
}

/// List cloud resources with optional filters
#[utoipa::path(
    get,
    path = "",
    tag = "cloud-resources",
    params(CloudResourceFilter),
    responses(
        (status = 200, description = "List of cloud resources", body = Vec<CloudResource>),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn list_cloud_resources<R>(
    State(service): State<Arc<CloudResourceService<R>>>,
    Query(filter): Query<CloudResourceFilter>,
) -> CloudResourceResult<impl IntoResponse>
where
    R: CloudResourceRepository,
{
    let resources = service.list(filter).await?;
    Ok(Json(resources))
}

/// List cloud resources by project ID
#[utoipa::path(
    get,
    path = "/project/{project_id}",
    tag = "cloud-resources",
    params(
        ("project_id" = Uuid, Path, description = "Project ID")
    ),
    responses(
        (status = 200, description = "List of cloud resources for project", body = Vec<CloudResource>),
        (status = 400, response = BadRequestUuidResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn list_by_project<R>(
    State(service): State<Arc<CloudResourceService<R>>>,
    UuidPath(project_id): UuidPath,
) -> CloudResourceResult<impl IntoResponse>
where
    R: CloudResourceRepository,
{
    let resources = service.list_by_project(project_id).await?;
    Ok(Json(resources))
}

/// Update a cloud resource
#[utoipa::path(
    put,
    path = "/{id}",
    tag = "cloud-resources",
    params(
        ("id" = Uuid, Path, description = "Cloud resource ID")
    ),
    request_body = UpdateCloudResource,
    responses(
        (status = 200, description = "Cloud resource updated successfully", body = CloudResource),
        (status = 400, response = BadRequestValidationResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn update_cloud_resource<R>(
    State(service): State<Arc<CloudResourceService<R>>>,
    UuidPath(id): UuidPath,
    Json(input): Json<UpdateCloudResource>,
) -> CloudResourceResult<impl IntoResponse>
where
    R: CloudResourceRepository,
{
    let resource = service.update(id, input).await?;
    Ok(Json(resource))
}

/// Delete a cloud resource (hard delete)
#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "cloud-resources",
    params(
        ("id" = Uuid, Path, description = "Cloud resource ID")
    ),
    responses(
        (status = 204, description = "Cloud resource deleted successfully"),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn delete_cloud_resource<R>(
    State(service): State<Arc<CloudResourceService<R>>>,
    headers: HeaderMap,
    UuidPath(id): UuidPath,
) -> CloudResourceResult<impl IntoResponse>
where
    R: CloudResourceRepository,
{
    service.delete(id).await?;

    // Audit log successful deletion
    AuditEvent::new(
        None, // TODO: Add user_id when authentication is implemented
        "cloud_resource.delete",
        Some(format!("cloud_resource:{}", id)),
        AuditOutcome::Success,
    )
    .with_ip(extract_ip_from_headers(&headers))
    .with_user_agent(extract_user_agent(&headers))
    .log();

    Ok(StatusCode::NO_CONTENT)
}

/// Soft delete a cloud resource
#[utoipa::path(
    post,
    path = "/{id}/soft-delete",
    tag = "cloud-resources",
    params(
        ("id" = Uuid, Path, description = "Cloud resource ID")
    ),
    responses(
        (status = 200, description = "Cloud resource soft deleted successfully", body = MessageResponse),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn soft_delete_cloud_resource<R>(
    State(service): State<Arc<CloudResourceService<R>>>,
    UuidPath(id): UuidPath,
) -> CloudResourceResult<impl IntoResponse>
where
    R: CloudResourceRepository,
{
    service.soft_delete(id).await?;
    Ok((
        StatusCode::OK,
        Json(MessageResponse {
            message: "Cloud resource soft deleted successfully".to_string(),
        }),
    ))
}
