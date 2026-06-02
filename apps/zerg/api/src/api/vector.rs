//! Vector API routes
//!
//! Exposes vector storage and search operations via REST API.

use axum::Router;

/// Create router for vector operations
///
/// Returns None if vector service is not configured
pub fn router(state: &crate::state::AppState) -> Option<Router> {
    state.vector_service.as_ref().map(|service| {
        // Clone the Arc to get a VectorService we can use
        // We need to create a new service instance since the handlers expect VectorService<R> not Arc<VectorService<R>>
        // Actually, the handlers module expects a VectorService directly, and wraps it in Arc internally
        // So we need to provide the inner service

        // Since VectorService doesn't implement Clone, and we have Arc<VectorService>,
        // we'll create the router directly here using the Arc

        use axum::{routing::get, routing::post};
        use std::sync::Arc;

        Router::new()
            // Collection management
            .route(
                "/collections",
                get(list_collections).post(create_collection),
            )
            .route(
                "/collections/{name}",
                get(get_collection).delete(delete_collection),
            )
            // Vector operations
            .route("/vectors/search", post(search))
            .route("/vectors/upsert", post(upsert))
            .route("/vectors/upsert-batch", post(upsert_batch))
            .route("/vectors/get", post(get_vectors))
            .route("/vectors/delete", post(delete_vectors))
            // Embedding operations
            .route("/embed", post(embed))
            .route("/search-with-embedding", post(search_with_embedding))
            .with_state(Arc::clone(service))
    })
}

// Re-export handlers with proper types
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use domain_vector::{
    QdrantRepository, VectorService,
    error::VectorResult,
    models::{
        CollectionInfo, CreateCollection, EmbeddingResult, SearchResult, TenantContext, Vector,
        VectorConfig,
    },
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

// Request DTOs
use domain_vector::handlers::direct::{
    CreateCollectionRequest, DeleteVectorsRequest, EmbedRequest, GetVectorsRequest, SearchRequest,
    SearchWithEmbeddingRequest, UpsertBatchRequest, UpsertRequest,
};

#[derive(Debug, Clone, Deserialize)]
pub struct TenantQueryParams {
    pub project_id: Uuid,
    pub namespace: Option<String>,
    pub user_id: Option<Uuid>,
}

// Collection handlers

pub async fn list_collections(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
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

pub async fn get_collection(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
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
        .ok_or(domain_vector::error::VectorError::CollectionNotFound(name))?;

    Ok(Json(collection))
}

pub async fn create_collection(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
    Json(request): Json<CreateCollectionRequest>,
) -> VectorResult<impl IntoResponse> {
    let input = CreateCollection {
        name: request.name,
        config: request.config.unwrap_or_else(|| VectorConfig::new(1536)),
    };

    let collection = service.create_collection(&request.tenant, input).await?;
    Ok((StatusCode::CREATED, Json(collection)))
}

pub async fn delete_collection(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
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

// Vector handlers

pub async fn search(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
    Json(request): Json<SearchRequest>,
) -> VectorResult<Json<Vec<SearchResult>>> {
    let results = service
        .search(&request.tenant, &request.collection_name, request.query)
        .await?;
    Ok(Json(results))
}

#[derive(serde::Serialize)]
pub struct UpsertResponse {
    pub id: Uuid,
    pub status: String,
}

pub async fn upsert(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
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

#[derive(serde::Serialize)]
pub struct UpsertBatchResponse {
    pub ids: Vec<Uuid>,
    pub count: u32,
    pub status: String,
}

pub async fn upsert_batch(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
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

pub async fn get_vectors(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
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

#[derive(serde::Serialize)]
pub struct DeleteResponse {
    pub deleted_count: u32,
    pub status: String,
}

pub async fn delete_vectors(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
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

// Embedding handlers

pub async fn embed(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
    Json(request): Json<EmbedRequest>,
) -> VectorResult<Json<EmbeddingResult>> {
    let result = service
        .embed(request.provider, request.model, &request.text)
        .await?;
    Ok(Json(result))
}

pub async fn search_with_embedding(
    State(service): State<Arc<VectorService<QdrantRepository>>>,
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
