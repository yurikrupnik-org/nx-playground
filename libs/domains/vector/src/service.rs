use std::sync::Arc;

use uuid::Uuid;

use crate::embedding::EmbeddingProvider;
use crate::error::{VectorError, VectorResult};
use crate::models::{
    CollectionInfo, CreateCollection, EmbeddingModel, EmbeddingResult, RecommendQuery,
    SearchQuery, SearchResult, TenantContext, Vector,
};
use crate::repository::VectorRepository;

/// Vector service providing high-level operations
///
/// Combines vector storage (Qdrant) with optional embedding generation.
///
/// The embedding provider is fixed at construction time via
/// [`VectorService::with_embedding_provider`]; per-call provider selection is
/// intentionally not supported (only a single provider is ever configured).
pub struct VectorService<R: VectorRepository> {
    repository: R,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

/// Parameters for [`VectorService::upsert_with_embedding`].
#[derive(Debug, Clone)]
pub struct UpsertWithEmbedding {
    pub id: Uuid,
    pub text: String,
    pub payload: Option<serde_json::Value>,
    pub model: EmbeddingModel,
    pub wait: bool,
}

/// Parameters for [`VectorService::search_with_embedding`].
#[derive(Debug, Clone)]
pub struct SearchWithEmbedding {
    pub text: String,
    pub limit: u32,
    pub score_threshold: Option<f32>,
    pub with_vectors: bool,
    pub with_payloads: bool,
    pub model: EmbeddingModel,
}

impl<R: VectorRepository> VectorService<R> {
    pub fn new(repository: R) -> Self {
        Self {
            repository,
            embedding_provider: None,
        }
    }

    pub fn with_embedding_provider(mut self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.embedding_provider = Some(provider);
        self
    }

    // ===== Collection Management =====

    pub async fn create_collection(
        &self,
        tenant: &TenantContext,
        input: CreateCollection,
    ) -> VectorResult<CollectionInfo> {
        self.repository.create_collection(tenant, input).await
    }

    pub async fn delete_collection(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
    ) -> VectorResult<bool> {
        self.repository
            .delete_collection(tenant, collection_name)
            .await
    }

    pub async fn get_collection(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
    ) -> VectorResult<Option<CollectionInfo>> {
        self.repository
            .get_collection(tenant, collection_name)
            .await
    }

    pub async fn list_collections(
        &self,
        tenant: &TenantContext,
    ) -> VectorResult<Vec<CollectionInfo>> {
        self.repository.list_collections(tenant).await
    }

    // ===== Vector Operations =====

    pub async fn upsert(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        vector: Vector,
        wait: bool,
    ) -> VectorResult<Uuid> {
        self.repository
            .upsert(tenant, collection_name, vector, wait)
            .await
    }

    pub async fn upsert_batch(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        vectors: Vec<Vector>,
        wait: bool,
    ) -> VectorResult<Vec<Uuid>> {
        self.repository
            .upsert_batch(tenant, collection_name, vectors, wait)
            .await
    }

    pub async fn search(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        query: SearchQuery,
    ) -> VectorResult<Vec<SearchResult>> {
        self.repository.search(tenant, collection_name, query).await
    }

    pub async fn get(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        ids: Vec<Uuid>,
        with_vectors: bool,
        with_payloads: bool,
    ) -> VectorResult<Vec<Vector>> {
        self.repository
            .get(tenant, collection_name, ids, with_vectors, with_payloads)
            .await
    }

    pub async fn delete(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        ids: Vec<Uuid>,
        wait: bool,
    ) -> VectorResult<u32> {
        self.repository
            .delete(tenant, collection_name, ids, wait)
            .await
    }

    // ===== Embedding Operations =====

    fn provider(&self) -> VectorResult<&Arc<dyn EmbeddingProvider>> {
        self.embedding_provider
            .as_ref()
            .ok_or_else(|| VectorError::Config("No embedding provider configured".to_string()))
    }

    pub async fn embed(&self, model: EmbeddingModel, text: &str) -> VectorResult<EmbeddingResult> {
        self.provider()?.embed(model, text).await
    }

    pub async fn embed_batch(
        &self,
        model: EmbeddingModel,
        texts: &[String],
    ) -> VectorResult<Vec<EmbeddingResult>> {
        self.provider()?.embed_batch(model, texts).await
    }

    // ===== Combined Operations =====

    /// Upsert a document with automatic embedding generation
    pub async fn upsert_with_embedding(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        params: UpsertWithEmbedding,
    ) -> VectorResult<Uuid> {
        let embedding = self.provider()?.embed(params.model, &params.text).await?;

        let mut vector = Vector::new(params.id, embedding.values);
        if let Some(p) = params.payload {
            vector = vector.with_payload(p);
        }

        self.repository
            .upsert(tenant, collection_name, vector, params.wait)
            .await
    }

    /// Search with automatic query embedding generation
    pub async fn search_with_embedding(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        params: SearchWithEmbedding,
    ) -> VectorResult<Vec<SearchResult>> {
        let embedding = self.provider()?.embed(params.model, &params.text).await?;

        let mut query = SearchQuery::new(embedding.values, params.limit);
        query.score_threshold = params.score_threshold;
        query.with_vectors = params.with_vectors;
        query.with_payloads = params.with_payloads;

        self.repository.search(tenant, collection_name, query).await
    }

    // ===== Recommendations =====

    pub async fn recommend(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        query: RecommendQuery,
    ) -> VectorResult<Vec<SearchResult>> {
        self.repository
            .recommend(tenant, collection_name, query)
            .await
    }
}
