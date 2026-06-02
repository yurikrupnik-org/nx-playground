//! Direct REST handlers for vector operations

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::VectorResult;
use crate::models::{
    CollectionInfo, CreateCollection, EmbeddingModel, EmbeddingProviderType, EmbeddingResult,
    SearchQuery, SearchResult, TenantContext, Vector, VectorConfig,
};
use crate::repository::VectorRepository;
use crate::service::VectorService;

// ===== Request/Response DTOs =====

/// Request to create a collection
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateCollectionRequest {
    pub tenant: TenantContext,
    pub name: String,
    #[serde(default)]
    pub config: Option<VectorConfig>,
}

/// Request to search vectors
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchRequest {
    pub tenant: TenantContext,
    pub collection_name: String,
    pub query: SearchQuery,
}

/// Request to upsert a single vector
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertRequest {
    pub tenant: TenantContext,
    pub collection_name: String,
    pub vector: Vector,
    #[serde(default)]
    pub wait: bool,
}

/// Request to upsert multiple vectors
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertBatchRequest {
    pub tenant: TenantContext,
    pub collection_name: String,
    pub vectors: Vec<Vector>,
    #[serde(default)]
    pub wait: bool,
}

/// Request to get vectors by IDs
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GetVectorsRequest {
    pub tenant: TenantContext,
    pub collection_name: String,
    pub ids: Vec<Uuid>,
    #[serde(default)]
    pub with_vectors: bool,
    #[serde(default = "default_true")]
    pub with_payloads: bool,
}

fn default_true() -> bool {
    true
}

/// Request to delete vectors
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteVectorsRequest {
    pub tenant: TenantContext,
    pub collection_name: String,
    pub ids: Vec<Uuid>,
    #[serde(default)]
    pub wait: bool,
}

/// Request to embed text
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbedRequest {
    pub text: String,
    #[serde(default)]
    pub provider: EmbeddingProviderType,
    #[serde(default)]
    pub model: EmbeddingModel,
}

/// Request to search with automatic embedding
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchWithEmbeddingRequest {
    pub tenant: TenantContext,
    pub collection_name: String,
    pub text: String,
    pub limit: u32,
    #[serde(default)]
    pub score_threshold: Option<f32>,
    #[serde(default)]
    pub with_vectors: bool,
    #[serde(default = "default_true")]
    pub with_payloads: bool,
    #[serde(default)]
    pub provider: EmbeddingProviderType,
    #[serde(default)]
    pub model: EmbeddingModel,
}

/// Response for upsert operations
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertResponse {
    pub id: Uuid,
    pub status: String,
}

/// Response for batch upsert operations
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertBatchResponse {
    pub ids: Vec<Uuid>,
    pub count: u32,
    pub status: String,
}

/// Response for delete operations
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteResponse {
    pub deleted_count: u32,
    pub status: String,
}

// ===== Collection Management =====

/// List all collections for a tenant
#[utoipa::path(
    get,
    path = "/collections",
    tag = "vector",
    params(
        ("project_id" = String, Query, description = "Project ID for tenant context"),
        ("namespace" = Option<String>, Query, description = "Optional namespace")
    ),
    responses(
        (status = 200, description = "List of collections", body = Vec<CollectionInfo>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_collections<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    axum::extract::Query(params): axum::extract::Query<TenantQueryParams>,
) -> VectorResult<Json<Vec<CollectionInfo>>> {
    let tenant = TenantContext {
        project_id: params.project_id,
        namespace: params.namespace,
        user_id: params.user_id,
    };

    let collections = service.list_collections(&tenant).await?;
    Ok(Json(collections))
}

#[derive(Debug, Clone, Deserialize)]
pub struct TenantQueryParams {
    pub project_id: Uuid,
    pub namespace: Option<String>,
    pub user_id: Option<Uuid>,
}

/// Get collection info
#[utoipa::path(
    get,
    path = "/collections/{name}",
    tag = "vector",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("project_id" = String, Query, description = "Project ID for tenant context"),
        ("namespace" = Option<String>, Query, description = "Optional namespace")
    ),
    responses(
        (status = 200, description = "Collection info", body = CollectionInfo),
        (status = 404, description = "Collection not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_collection<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<TenantQueryParams>,
) -> VectorResult<impl IntoResponse> {
    let tenant = TenantContext {
        project_id: params.project_id,
        namespace: params.namespace,
        user_id: params.user_id,
    };

    let collection = service
        .get_collection(&tenant, &name)
        .await?
        .ok_or(crate::error::VectorError::CollectionNotFound(name))?;

    Ok(Json(collection))
}

/// Create a new collection
#[utoipa::path(
    post,
    path = "/collections",
    tag = "vector",
    request_body = CreateCollectionRequest,
    responses(
        (status = 201, description = "Collection created", body = CollectionInfo),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn create_collection<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    Json(request): Json<CreateCollectionRequest>,
) -> VectorResult<impl IntoResponse> {
    let input = CreateCollection {
        name: request.name,
        config: request.config.unwrap_or_else(|| VectorConfig::new(1536)),
    };

    let collection = service.create_collection(&request.tenant, input).await?;
    Ok((StatusCode::CREATED, Json(collection)))
}

/// Delete a collection
#[utoipa::path(
    delete,
    path = "/collections/{name}",
    tag = "vector",
    params(
        ("name" = String, Path, description = "Collection name"),
        ("project_id" = String, Query, description = "Project ID for tenant context"),
        ("namespace" = Option<String>, Query, description = "Optional namespace")
    ),
    responses(
        (status = 204, description = "Collection deleted"),
        (status = 404, description = "Collection not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_collection<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<TenantQueryParams>,
) -> VectorResult<impl IntoResponse> {
    let tenant = TenantContext {
        project_id: params.project_id,
        namespace: params.namespace,
        user_id: params.user_id,
    };

    service.delete_collection(&tenant, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ===== Vector Operations =====

/// Search vectors
#[utoipa::path(
    post,
    path = "/vectors/search",
    tag = "vector",
    request_body = SearchRequest,
    responses(
        (status = 200, description = "Search results", body = Vec<SearchResult>),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn search<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    Json(request): Json<SearchRequest>,
) -> VectorResult<Json<Vec<SearchResult>>> {
    let results = service
        .search(&request.tenant, &request.collection_name, request.query)
        .await?;
    Ok(Json(results))
}

/// Upsert a single vector
#[utoipa::path(
    post,
    path = "/vectors/upsert",
    tag = "vector",
    request_body = UpsertRequest,
    responses(
        (status = 200, description = "Vector upserted", body = UpsertResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn upsert<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    Json(request): Json<UpsertRequest>,
) -> VectorResult<Json<UpsertResponse>> {
    let id = service
        .upsert(
            &request.tenant,
            &request.collection_name,
            request.vector,
            request.wait,
        )
        .await?;

    Ok(Json(UpsertResponse {
        id,
        status: if request.wait {
            "completed".to_string()
        } else {
            "pending".to_string()
        },
    }))
}

/// Upsert multiple vectors
#[utoipa::path(
    post,
    path = "/vectors/upsert-batch",
    tag = "vector",
    request_body = UpsertBatchRequest,
    responses(
        (status = 200, description = "Vectors upserted", body = UpsertBatchResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn upsert_batch<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    Json(request): Json<UpsertBatchRequest>,
) -> VectorResult<Json<UpsertBatchResponse>> {
    let count = request.vectors.len() as u32;
    let ids = service
        .upsert_batch(
            &request.tenant,
            &request.collection_name,
            request.vectors,
            request.wait,
        )
        .await?;

    Ok(Json(UpsertBatchResponse {
        ids,
        count,
        status: if request.wait {
            "completed".to_string()
        } else {
            "pending".to_string()
        },
    }))
}

/// Get vectors by IDs
#[utoipa::path(
    post,
    path = "/vectors/get",
    tag = "vector",
    request_body = GetVectorsRequest,
    responses(
        (status = 200, description = "Retrieved vectors", body = Vec<Vector>),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_vectors<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    Json(request): Json<GetVectorsRequest>,
) -> VectorResult<Json<Vec<Vector>>> {
    let vectors = service
        .get(
            &request.tenant,
            &request.collection_name,
            request.ids,
            request.with_vectors,
            request.with_payloads,
        )
        .await?;
    Ok(Json(vectors))
}

/// Delete vectors by IDs
#[utoipa::path(
    post,
    path = "/vectors/delete",
    tag = "vector",
    request_body = DeleteVectorsRequest,
    responses(
        (status = 200, description = "Vectors deleted", body = DeleteResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_vectors<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    Json(request): Json<DeleteVectorsRequest>,
) -> VectorResult<Json<DeleteResponse>> {
    let deleted_count = service
        .delete(
            &request.tenant,
            &request.collection_name,
            request.ids,
            request.wait,
        )
        .await?;

    Ok(Json(DeleteResponse {
        deleted_count,
        status: if request.wait {
            "completed".to_string()
        } else {
            "pending".to_string()
        },
    }))
}

// ===== Embedding Operations =====

/// Generate embeddings for text
#[utoipa::path(
    post,
    path = "/embed",
    tag = "vector",
    request_body = EmbedRequest,
    responses(
        (status = 200, description = "Embedding generated", body = EmbeddingResult),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn embed<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    Json(request): Json<EmbedRequest>,
) -> VectorResult<Json<EmbeddingResult>> {
    let result = service
        .embed(request.provider, request.model, &request.text)
        .await?;
    Ok(Json(result))
}

/// Search with automatic query embedding
#[utoipa::path(
    post,
    path = "/search-with-embedding",
    tag = "vector",
    request_body = SearchWithEmbeddingRequest,
    responses(
        (status = 200, description = "Search results", body = Vec<SearchResult>),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn search_with_embedding<R: VectorRepository>(
    State(service): State<Arc<VectorService<R>>>,
    Json(request): Json<SearchWithEmbeddingRequest>,
) -> VectorResult<Json<Vec<SearchResult>>> {
    let results = service
        .search_with_embedding(
            &request.tenant,
            &request.collection_name,
            &request.text,
            request.limit,
            request.score_threshold,
            request.with_vectors,
            request.with_payloads,
            request.provider,
            request.model,
        )
        .await?;
    Ok(Json(results))
}
