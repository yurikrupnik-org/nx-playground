use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use axum_helpers::{
    ValidatedJson,
    errors::responses::{
        BadRequestUuidResponse, BadRequestValidationResponse, InternalServerErrorResponse,
        NotFoundResponse,
    },
};
use serde::Serialize;
use std::sync::Arc;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use crate::error::UserResult;
use crate::models::{CreateUser, UpdateUser, UserFilter, UserResponse};
use crate::repository::UserRepository;
use crate::service::UserService;

/// OpenAPI documentation for Users API
#[derive(OpenApi)]
#[openapi(
    paths(
        list_users,
        create_user,
        get_user,
        update_user,
        delete_user,
        verify_email,
    ),
    components(
        schemas(
            UserResponse,
            CreateUser,
            UpdateUser,
            UserFilter,
            ListUsersResponse,
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
        (name = "users", description = "User management endpoints")
    )
)]
pub struct ApiDoc;

/// Create the users router with all HTTP endpoints
pub fn router<R: UserRepository + 'static>(service: UserService<R>) -> Router {
    let shared_service = Arc::new(service);

    Router::new()
        .route("/", get(list_users).post(create_user))
        .route("/{id}", get(get_user).put(update_user).delete(delete_user))
        .route("/{id}/verify-email", post(verify_email))
        .with_state(shared_service)
}

/// List response with pagination info
#[derive(Debug, Serialize, ToSchema)]
pub struct ListUsersResponse {
    pub data: Vec<UserResponse>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

/// List users with optional filters
#[utoipa::path(
    get,
    path = "",
    tag = "users",
    params(UserFilter),
    responses(
        (status = 200, description = "List of users with pagination", body = ListUsersResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn list_users<R: UserRepository>(
    State(service): State<Arc<UserService<R>>>,
    Query(filter): Query<UserFilter>,
) -> UserResult<Json<ListUsersResponse>> {
    let limit = filter.limit;
    let offset = filter.offset;
    let (users, total) = service.list_users(filter).await?;

    Ok(Json(ListUsersResponse {
        data: users,
        total,
        limit,
        offset,
    }))
}

/// Create a new user
#[utoipa::path(
    post,
    path = "",
    tag = "users",
    request_body = CreateUser,
    responses(
        (status = 201, description = "User created successfully", body = UserResponse),
        (status = 400, response = BadRequestValidationResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn create_user<R: UserRepository>(
    State(service): State<Arc<UserService<R>>>,
    ValidatedJson(input): ValidatedJson<CreateUser>,
) -> UserResult<impl IntoResponse> {
    let user = service.create_user(input).await?;
    Ok((StatusCode::CREATED, Json(user)))
}

/// Get a user by ID
#[utoipa::path(
    get,
    path = "/{id}",
    tag = "users",
    params(
        ("id" = Uuid, Path, description = "User ID")
    ),
    responses(
        (status = 200, description = "User found", body = UserResponse),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn get_user<R: UserRepository>(
    State(service): State<Arc<UserService<R>>>,
    Path(id): Path<Uuid>,
) -> UserResult<Json<UserResponse>> {
    let user = service.get_user(id).await?;
    Ok(Json(user))
}

/// Update a user
#[utoipa::path(
    put,
    path = "/{id}",
    tag = "users",
    params(
        ("id" = Uuid, Path, description = "User ID")
    ),
    request_body = UpdateUser,
    responses(
        (status = 200, description = "User updated successfully", body = UserResponse),
        (status = 400, response = BadRequestValidationResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn update_user<R: UserRepository>(
    State(service): State<Arc<UserService<R>>>,
    Path(id): Path<Uuid>,
    ValidatedJson(input): ValidatedJson<UpdateUser>,
) -> UserResult<Json<UserResponse>> {
    let user = service.update_user(id, input).await?;
    Ok(Json(user))
}

/// Delete a user
#[utoipa::path(
    delete,
    path = "/{id}",
    tag = "users",
    params(
        ("id" = Uuid, Path, description = "User ID")
    ),
    responses(
        (status = 204, description = "User deleted successfully"),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn delete_user<R: UserRepository>(
    State(service): State<Arc<UserService<R>>>,
    Path(id): Path<Uuid>,
) -> UserResult<impl IntoResponse> {
    service.delete_user(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Verify user email
#[utoipa::path(
    post,
    path = "/{id}/verify-email",
    tag = "users",
    params(
        ("id" = Uuid, Path, description = "User ID")
    ),
    responses(
        (status = 200, description = "Email verified successfully", body = UserResponse),
        (status = 400, response = BadRequestUuidResponse),
        (status = 404, response = NotFoundResponse),
        (status = 500, response = InternalServerErrorResponse)
    )
)]
async fn verify_email<R: UserRepository>(
    State(service): State<Arc<UserService<R>>>,
    Path(id): Path<Uuid>,
) -> UserResult<Json<UserResponse>> {
    let user = service.verify_email(id).await?;
    Ok(Json(user))
}

/// Change password response
#[derive(Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}
