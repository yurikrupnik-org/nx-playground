//! HTTP handlers for vector operations
//!
//! Provides REST API handlers for vector storage operations.

pub mod direct;

use axum::{Router, routing::get, routing::post};
use std::sync::Arc;
use utoipa::OpenApi;

use crate::models::{
    CollectionInfo, CreateCollection, EmbeddingResult, SearchQuery, SearchResult, TenantContext,
    Vector, VectorConfig,
};
use crate::repository::VectorRepository;
use crate::service::VectorService;

/// OpenAPI documentation for Vector API
#[derive(OpenApi)]
#[openapi(
    paths(
        direct::list_collections,
        direct::get_collection,
        direct::create_collection,
        direct::delete_collection,
        direct::search,
        direct::upsert,
        direct::upsert_batch,
        direct::get_vectors,
        direct::delete_vectors,
        direct::embed,
        direct::search_with_embedding,
    ),
    components(
        schemas(
            CollectionInfo,
            CreateCollection,
            Vector,
            VectorConfig,
            SearchQuery,
            SearchResult,
            TenantContext,
            EmbeddingResult,
            direct::CreateCollectionRequest,
            direct::SearchRequest,
            direct::UpsertRequest,
            direct::UpsertBatchRequest,
            direct::GetVectorsRequest,
            direct::DeleteVectorsRequest,
            direct::EmbedRequest,
            direct::SearchWithEmbeddingRequest,
        )
    ),
    tags(
        (name = "vector", description = "Vector storage and search operations")
    )
)]
pub struct VectorApiDoc;

/// Create router for direct Qdrant-backed handlers
pub fn router<R: VectorRepository + 'static>(service: VectorService<R>) -> Router {
    let shared_service = Arc::new(service);

    Router::new()
        // Collection management
        .route(
            "/collections",
            get(direct::list_collections).post(direct::create_collection),
        )
        .route(
            "/collections/{name}",
            get(direct::get_collection).delete(direct::delete_collection),
        )
        // Vector operations
        .route("/vectors/search", post(direct::search))
        .route("/vectors/upsert", post(direct::upsert))
        .route("/vectors/upsert-batch", post(direct::upsert_batch))
        .route("/vectors/get", post(direct::get_vectors))
        .route("/vectors/delete", post(direct::delete_vectors))
        // Embedding operations
        .route("/embed", post(direct::embed))
        .route(
            "/search-with-embedding",
            post(direct::search_with_embedding),
        )
        .with_state(shared_service)
}
