#![allow(clippy::result_large_err)]

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::VectorResult;
use crate::models::{
    CollectionInfo, CreateCollection, RecommendQuery, SearchQuery, SearchResult, TenantContext,
    Vector,
};

/// Repository trait for vector storage operations
///
/// This trait abstracts the underlying vector database (Qdrant).
/// All collection names are automatically prefixed with tenant context.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait VectorRepository: Send + Sync {
    // ===== Collection Management =====

    /// Create a new collection with the given configuration
    async fn create_collection(
        &self,
        tenant: &TenantContext,
        input: CreateCollection,
    ) -> VectorResult<CollectionInfo>;

    /// Delete a collection
    async fn delete_collection(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
    ) -> VectorResult<bool>;

    /// Get collection info
    async fn get_collection(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
    ) -> VectorResult<Option<CollectionInfo>>;

    /// List all collections for a tenant
    async fn list_collections(&self, tenant: &TenantContext) -> VectorResult<Vec<CollectionInfo>>;

    // ===== Vector Operations =====

    /// Upsert a single vector
    async fn upsert(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        vector: Vector,
        wait: bool,
    ) -> VectorResult<Uuid>;

    /// Upsert multiple vectors in batch
    async fn upsert_batch(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        vectors: Vec<Vector>,
        wait: bool,
    ) -> VectorResult<Vec<Uuid>>;

    /// Search for similar vectors
    async fn search(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        query: SearchQuery,
    ) -> VectorResult<Vec<SearchResult>>;

    /// Get vectors by IDs
    async fn get(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        ids: Vec<Uuid>,
        with_vectors: bool,
        with_payloads: bool,
    ) -> VectorResult<Vec<Vector>>;

    /// Delete vectors by IDs
    async fn delete(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        ids: Vec<Uuid>,
        wait: bool,
    ) -> VectorResult<u32>;

    // ===== Recommendations =====

    /// Get recommendations based on positive and negative examples
    async fn recommend(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        query: RecommendQuery,
    ) -> VectorResult<Vec<SearchResult>>;
}
