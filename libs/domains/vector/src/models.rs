use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Tenant context for multi-tenancy support
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TenantContext {
    pub project_id: Uuid,
    pub namespace: Option<String>,
    pub user_id: Option<Uuid>,
}

impl TenantContext {
    pub fn new(project_id: Uuid) -> Self {
        Self {
            project_id,
            namespace: None,
            user_id: None,
        }
    }

    pub fn with_namespace(mut self, namespace: String) -> Self {
        self.namespace = Some(namespace);
        self
    }

    pub fn with_user(mut self, user_id: Uuid) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Generate tenant-prefixed collection name for isolation
    pub fn collection_name(&self, base_name: &str) -> String {
        match &self.namespace {
            Some(ns) => format!("{}_{}_{}", self.project_id, ns, base_name),
            None => format!("{}_{}", self.project_id, base_name),
        }
    }
}

/// Distance metric for similarity calculations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub enum DistanceMetric {
    #[default]
    Cosine,
    Euclidean,
    DotProduct,
    Manhattan,
}

/// HNSW index configuration
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HnswConfig {
    pub m: Option<u32>,
    pub ef_construct: Option<u32>,
    pub full_scan_threshold: Option<u32>,
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self {
            m: Some(16),
            ef_construct: Some(100),
            full_scan_threshold: None,
        }
    }
}

/// Vector collection configuration
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VectorConfig {
    pub dimension: u32,
    pub distance: DistanceMetric,
    pub hnsw: Option<HnswConfig>,
}

impl VectorConfig {
    pub fn new(dimension: u32) -> Self {
        Self {
            dimension,
            distance: DistanceMetric::default(),
            hnsw: None,
        }
    }

    pub fn with_distance(mut self, distance: DistanceMetric) -> Self {
        self.distance = distance;
        self
    }

    pub fn with_hnsw(mut self, hnsw: HnswConfig) -> Self {
        self.hnsw = Some(hnsw);
        self
    }
}

/// Input for creating a collection
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateCollection {
    pub name: String,
    pub config: VectorConfig,
}

/// Collection information
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CollectionInfo {
    pub name: String,
    /// Approximate number of vectors, when reported by the backend.
    ///
    /// Qdrant >= 1.18 no longer reports a separate vector count, so this is
    /// `None` for collections read back from Qdrant.
    pub vectors_count: Option<u64>,
    /// Approximate number of indexed vectors; `None` when not reported.
    pub indexed_vectors_count: Option<u64>,
    /// Approximate number of points; `None` when not reported.
    pub points_count: Option<u64>,
    pub config: VectorConfig,
    pub status: CollectionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum CollectionStatus {
    Green,
    Yellow,
    Grey,
}

impl CollectionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CollectionStatus::Green => "green",
            CollectionStatus::Yellow => "yellow",
            CollectionStatus::Grey => "grey",
        }
    }
}

/// A vector point with payload
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Vector {
    pub id: Uuid,
    pub values: Vec<f32>,
    pub payload: Option<serde_json::Value>,
    pub sparse: Option<SparseVector>,
}

impl Vector {
    pub fn new(id: Uuid, values: Vec<f32>) -> Self {
        Self {
            id,
            values,
            payload: None,
            sparse: None,
        }
    }

    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

/// Sparse vector for hybrid search
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SparseVector {
    pub indices: Vec<u32>,
    pub values: Vec<f32>,
}

/// Search query parameters
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchQuery {
    pub vector: Vec<f32>,
    pub limit: u32,
    pub score_threshold: Option<f32>,
    pub filter: Option<SearchFilter>,
    pub with_vectors: bool,
    pub with_payloads: bool,
}

impl SearchQuery {
    pub fn new(vector: Vec<f32>, limit: u32) -> Self {
        Self {
            vector,
            limit,
            score_threshold: None,
            filter: None,
            with_vectors: false,
            with_payloads: true,
        }
    }
}

/// Search filter conditions
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct SearchFilter {
    pub must_have_id: Option<Uuid>,
    pub must_match: Option<serde_json::Value>,
    pub namespace_filter: Option<String>,
}

/// Search result
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchResult {
    pub id: Uuid,
    pub score: f32,
    pub payload: Option<serde_json::Value>,
    pub vector: Option<Vec<f32>>,
}

impl SearchResult {
    pub fn new(
        id: Uuid,
        score: f32,
        payload: Option<serde_json::Value>,
        vector: Option<Vec<f32>>,
    ) -> Self {
        Self {
            id,
            score,
            payload,
            vector,
        }
    }
}

/// Embedding provider types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub enum EmbeddingProviderType {
    #[default]
    OpenAI,
    Anthropic,
    Local,
    VertexAI,
    Cohere,
    Voyage,
}

/// Embedding model selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub enum EmbeddingModel {
    // OpenAI models
    /// OpenAI text-embedding-3-small (1536 dimensions)
    #[default]
    TextEmbedding3Small,
    /// OpenAI text-embedding-3-large (3072 dimensions)
    TextEmbedding3Large,
    /// OpenAI text-embedding-ada-002 (1536 dimensions, legacy)
    TextEmbeddingAda002,

    // Vertex AI models
    /// Vertex AI textembedding-gecko (768 dimensions)
    Gecko,
    /// Vertex AI textembedding-gecko-multilingual (768 dimensions)
    GeckoMultilingual,
    /// Vertex AI text-embedding-004 (768 dimensions)
    TextEmbedding004,
    /// Vertex AI text-embedding-005 (768 dimensions)
    TextEmbedding005,
    /// Vertex AI text-multilingual-embedding-002 (768 dimensions)
    TextMultilingualEmbedding002,

    // Cohere models
    /// Cohere embed-english-v3.0 (1024 dimensions)
    CohereEmbedV3,
    /// Cohere embed-multilingual-v3.0 (1024 dimensions)
    CohereEmbedMultilingualV3,

    // Voyage AI models
    /// Voyage voyage-3 (1024 dimensions)
    Voyage3,
    /// Voyage voyage-3-lite (512 dimensions)
    Voyage3Lite,
    /// Voyage voyage-code-3 (1024 dimensions)
    VoyageCode3,

    /// Custom model with specified dimension
    Custom(u32),
}

impl EmbeddingModel {
    pub fn dimension(&self) -> u32 {
        match self {
            // OpenAI
            EmbeddingModel::TextEmbedding3Small => 1536,
            EmbeddingModel::TextEmbedding3Large => 3072,
            EmbeddingModel::TextEmbeddingAda002 => 1536,
            // Vertex AI
            EmbeddingModel::Gecko => 768,
            EmbeddingModel::GeckoMultilingual => 768,
            EmbeddingModel::TextEmbedding004 => 768,
            EmbeddingModel::TextEmbedding005 => 768,
            EmbeddingModel::TextMultilingualEmbedding002 => 768,
            // Cohere
            EmbeddingModel::CohereEmbedV3 => 1024,
            EmbeddingModel::CohereEmbedMultilingualV3 => 1024,
            // Voyage
            EmbeddingModel::Voyage3 => 1024,
            EmbeddingModel::Voyage3Lite => 512,
            EmbeddingModel::VoyageCode3 => 1024,
            // Custom
            EmbeddingModel::Custom(dim) => *dim,
        }
    }

    pub fn model_name(&self) -> &'static str {
        match self {
            // OpenAI
            EmbeddingModel::TextEmbedding3Small => "text-embedding-3-small",
            EmbeddingModel::TextEmbedding3Large => "text-embedding-3-large",
            EmbeddingModel::TextEmbeddingAda002 => "text-embedding-ada-002",
            // Vertex AI
            EmbeddingModel::Gecko => "textembedding-gecko@003",
            EmbeddingModel::GeckoMultilingual => "textembedding-gecko-multilingual@001",
            EmbeddingModel::TextEmbedding004 => "text-embedding-004",
            EmbeddingModel::TextEmbedding005 => "text-embedding-005",
            EmbeddingModel::TextMultilingualEmbedding002 => "text-multilingual-embedding-002",
            // Cohere
            EmbeddingModel::CohereEmbedV3 => "embed-english-v3.0",
            EmbeddingModel::CohereEmbedMultilingualV3 => "embed-multilingual-v3.0",
            // Voyage
            EmbeddingModel::Voyage3 => "voyage-3",
            EmbeddingModel::Voyage3Lite => "voyage-3-lite",
            EmbeddingModel::VoyageCode3 => "voyage-code-3",
            // Custom
            EmbeddingModel::Custom(_) => "custom",
        }
    }

    /// Get the provider type this model belongs to
    pub fn provider(&self) -> EmbeddingProviderType {
        match self {
            EmbeddingModel::TextEmbedding3Small
            | EmbeddingModel::TextEmbedding3Large
            | EmbeddingModel::TextEmbeddingAda002 => EmbeddingProviderType::OpenAI,
            EmbeddingModel::Gecko
            | EmbeddingModel::GeckoMultilingual
            | EmbeddingModel::TextEmbedding004
            | EmbeddingModel::TextEmbedding005
            | EmbeddingModel::TextMultilingualEmbedding002 => EmbeddingProviderType::VertexAI,
            EmbeddingModel::CohereEmbedV3 | EmbeddingModel::CohereEmbedMultilingualV3 => {
                EmbeddingProviderType::Cohere
            }
            EmbeddingModel::Voyage3 | EmbeddingModel::Voyage3Lite | EmbeddingModel::VoyageCode3 => {
                EmbeddingProviderType::Voyage
            }
            EmbeddingModel::Custom(_) => EmbeddingProviderType::Local,
        }
    }
}

/// Embedding result
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbeddingResult {
    pub values: Vec<f32>,
    pub dimension: u32,
    pub tokens_used: u32,
}

/// Recommendation request
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RecommendQuery {
    pub positive_ids: Vec<Uuid>,
    pub negative_ids: Vec<Uuid>,
    pub limit: u32,
    pub score_threshold: Option<f32>,
    pub filter: Option<SearchFilter>,
    pub with_vectors: bool,
    pub with_payloads: bool,
}
