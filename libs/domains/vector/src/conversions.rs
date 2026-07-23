use uuid::Uuid;

use crate::error::{VectorError, VectorResult};
use crate::models::{
    CollectionInfo, DistanceMetric, EmbeddingModel, EmbeddingProviderType, HnswConfig,
    SearchResult, TenantContext, Vector, VectorConfig,
};

// Import generated proto types
use rpc::vector::v1::{
    CollectionInfo as ProtoCollectionInfo, DistanceMetric as ProtoDistance,
    EmbeddingModel as ProtoEmbeddingModel, EmbeddingProvider as ProtoEmbeddingProvider,
    HnswConfig as ProtoHnswConfig, Payload as ProtoPayload, RecommendResponse, SearchResponse,
    SearchResult as ProtoSearchResult, TenantContext as ProtoTenantContext, Vector as ProtoVector,
    VectorConfig as ProtoVectorConfig,
};

// ===== Tenant Context =====

/// Convert an optional proto tenant context, erroring when it is missing.
pub fn tenant_from_proto(proto: Option<ProtoTenantContext>) -> VectorResult<TenantContext> {
    proto
        .ok_or_else(|| VectorError::Validation("Missing tenant context".to_string()))?
        .try_into()
}

impl TryFrom<ProtoTenantContext> for TenantContext {
    type Error = VectorError;

    fn try_from(proto: ProtoTenantContext) -> Result<Self, Self::Error> {
        let project_id = bytes_to_uuid(&proto.project_id)?;
        let namespace = proto.namespace.filter(|s| !s.is_empty());
        let user_id = match proto.user_id {
            Some(ref bytes) if !bytes.is_empty() => Some(bytes_to_uuid(bytes)?),
            _ => None,
        };

        Ok(TenantContext {
            project_id,
            namespace,
            user_id,
        })
    }
}

impl From<TenantContext> for ProtoTenantContext {
    fn from(ctx: TenantContext) -> Self {
        ProtoTenantContext {
            project_id: ctx.project_id.as_bytes().to_vec(),
            namespace: ctx.namespace,
            user_id: ctx.user_id.map(|id| id.as_bytes().to_vec()),
        }
    }
}

// ===== Distance Metric =====

pub fn distance_from_proto(proto: i32) -> VectorResult<DistanceMetric> {
    match ProtoDistance::try_from(proto) {
        Ok(ProtoDistance::Cosine) => Ok(DistanceMetric::Cosine),
        Ok(ProtoDistance::Euclidean) => Ok(DistanceMetric::Euclidean),
        Ok(ProtoDistance::DotProduct) => Ok(DistanceMetric::DotProduct),
        Ok(ProtoDistance::Manhattan) => Ok(DistanceMetric::Manhattan),
        _ => Err(VectorError::Validation(format!(
            "Unknown distance metric: {proto}"
        ))),
    }
}

pub fn distance_to_proto(metric: DistanceMetric) -> i32 {
    match metric {
        DistanceMetric::Cosine => ProtoDistance::Cosine as i32,
        DistanceMetric::Euclidean => ProtoDistance::Euclidean as i32,
        DistanceMetric::DotProduct => ProtoDistance::DotProduct as i32,
        DistanceMetric::Manhattan => ProtoDistance::Manhattan as i32,
    }
}

// ===== HNSW Config =====

pub fn hnsw_from_proto(proto: Option<ProtoHnswConfig>) -> Option<HnswConfig> {
    proto.map(|h| HnswConfig {
        m: h.m,
        ef_construct: h.ef_construct,
        full_scan_threshold: h.full_scan_threshold,
    })
}

pub fn hnsw_to_proto(hnsw: Option<HnswConfig>) -> Option<ProtoHnswConfig> {
    hnsw.map(|h| ProtoHnswConfig {
        m: h.m,
        ef_construct: h.ef_construct,
        full_scan_threshold: h.full_scan_threshold,
    })
}

// ===== Vector Config =====

pub fn vector_config_from_proto(proto: Option<ProtoVectorConfig>) -> VectorResult<VectorConfig> {
    let config =
        proto.ok_or_else(|| VectorError::Validation("Missing vector config".to_string()))?;
    Ok(VectorConfig {
        dimension: config.dimension,
        distance: distance_from_proto(config.distance)?,
        hnsw: hnsw_from_proto(config.hnsw),
    })
}

impl From<VectorConfig> for ProtoVectorConfig {
    fn from(config: VectorConfig) -> Self {
        ProtoVectorConfig {
            dimension: config.dimension,
            distance: distance_to_proto(config.distance),
            hnsw: hnsw_to_proto(config.hnsw),
        }
    }
}

// ===== Collection Info =====

impl From<CollectionInfo> for ProtoCollectionInfo {
    fn from(info: CollectionInfo) -> Self {
        ProtoCollectionInfo {
            collection_name: info.name,
            // proto3 cannot express absence for these counters; 0 means
            // "not reported by the backend".
            vectors_count: info.vectors_count.unwrap_or(0),
            indexed_vectors_count: info.indexed_vectors_count.unwrap_or(0),
            points_count: info.points_count.unwrap_or(0),
            config: Some(info.config.into()),
            status: info.status.as_str().to_string(),
        }
    }
}

// ===== Vector =====

/// Convert an optional proto vector, erroring when it is missing.
pub fn vector_from_proto(proto: Option<ProtoVector>) -> VectorResult<Vector> {
    proto
        .ok_or_else(|| VectorError::Validation("Missing vector".to_string()))?
        .try_into()
}

impl TryFrom<ProtoVector> for Vector {
    type Error = VectorError;

    fn try_from(proto: ProtoVector) -> Result<Self, Self::Error> {
        let id = bytes_to_uuid(&proto.id)?;
        let payload = payload_from_proto(proto.payload)?;

        Ok(Vector {
            id,
            values: proto.values,
            payload,
            sparse: None,
        })
    }
}

/// Parse an optional proto payload into a JSON value.
///
/// An absent or empty payload is `None`; malformed JSON is a validation error
/// rather than being silently dropped.
pub fn payload_from_proto(proto: Option<ProtoPayload>) -> VectorResult<Option<serde_json::Value>> {
    match proto {
        Some(p) if !p.json.is_empty() => serde_json::from_slice(&p.json)
            .map(Some)
            .map_err(|e| VectorError::Validation(format!("Invalid payload JSON: {e}"))),
        _ => Ok(None),
    }
}

/// Serialize a JSON payload for the wire.
///
/// Serializing a `serde_json::Value` cannot realistically fail (object keys
/// are always strings); a failure is logged and mapped to an empty payload.
fn payload_to_proto(payload: Option<serde_json::Value>) -> Option<ProtoPayload> {
    payload.map(|p| ProtoPayload {
        json: serde_json::to_vec(&p).unwrap_or_else(|e| {
            tracing::warn!("Failed to serialize payload to proto: {e}");
            Vec::new()
        }),
    })
}

impl From<Vector> for ProtoVector {
    fn from(vector: Vector) -> Self {
        ProtoVector {
            id: vector.id.as_bytes().to_vec(),
            values: vector.values,
            payload: payload_to_proto(vector.payload),
            sparse: None,
        }
    }
}

// ===== Search Result =====

impl From<SearchResult> for ProtoSearchResult {
    fn from(result: SearchResult) -> Self {
        ProtoSearchResult {
            id: result.id.as_bytes().to_vec(),
            score: result.score,
            payload: payload_to_proto(result.payload),
            vector: result.vector.map(|values| ProtoVector {
                id: result.id.as_bytes().to_vec(),
                values,
                payload: None,
                sparse: None,
            }),
        }
    }
}

pub fn search_results_to_response(results: Vec<SearchResult>) -> SearchResponse {
    SearchResponse {
        results: results.into_iter().map(Into::into).collect(),
        search_time_ms: 0,
    }
}

pub fn search_results_to_recommend_response(results: Vec<SearchResult>) -> RecommendResponse {
    RecommendResponse {
        results: results.into_iter().map(Into::into).collect(),
        search_time_ms: 0,
    }
}

// ===== Embedding Provider =====

pub fn embedding_provider_from_proto(proto: i32) -> VectorResult<EmbeddingProviderType> {
    match ProtoEmbeddingProvider::try_from(proto) {
        Ok(ProtoEmbeddingProvider::EmbeddingOpenai) => Ok(EmbeddingProviderType::OpenAI),
        Ok(ProtoEmbeddingProvider::EmbeddingAnthropic) => Ok(EmbeddingProviderType::Anthropic),
        Ok(ProtoEmbeddingProvider::EmbeddingLocal) => Ok(EmbeddingProviderType::Local),
        Ok(ProtoEmbeddingProvider::EmbeddingVertexai) => Ok(EmbeddingProviderType::VertexAI),
        Ok(ProtoEmbeddingProvider::EmbeddingCohere) => Ok(EmbeddingProviderType::Cohere),
        Ok(ProtoEmbeddingProvider::EmbeddingVoyage) => Ok(EmbeddingProviderType::Voyage),
        _ => Err(VectorError::Validation(format!(
            "Unknown embedding provider: {proto}"
        ))),
    }
}

pub fn embedding_provider_to_proto(provider: EmbeddingProviderType) -> i32 {
    match provider {
        EmbeddingProviderType::OpenAI => ProtoEmbeddingProvider::EmbeddingOpenai as i32,
        EmbeddingProviderType::Anthropic => ProtoEmbeddingProvider::EmbeddingAnthropic as i32,
        EmbeddingProviderType::Local => ProtoEmbeddingProvider::EmbeddingLocal as i32,
        EmbeddingProviderType::VertexAI => ProtoEmbeddingProvider::EmbeddingVertexai as i32,
        EmbeddingProviderType::Cohere => ProtoEmbeddingProvider::EmbeddingCohere as i32,
        EmbeddingProviderType::Voyage => ProtoEmbeddingProvider::EmbeddingVoyage as i32,
    }
}

// ===== Embedding Model =====

pub fn embedding_model_from_proto(
    proto: i32,
    custom_dim: Option<u32>,
) -> VectorResult<EmbeddingModel> {
    match ProtoEmbeddingModel::try_from(proto) {
        // OpenAI models
        Ok(ProtoEmbeddingModel::Embedding3Small) => Ok(EmbeddingModel::TextEmbedding3Small),
        Ok(ProtoEmbeddingModel::Embedding3Large) => Ok(EmbeddingModel::TextEmbedding3Large),
        Ok(ProtoEmbeddingModel::EmbeddingAda002) => Ok(EmbeddingModel::TextEmbeddingAda002),
        // Vertex AI models
        Ok(ProtoEmbeddingModel::Gecko) => Ok(EmbeddingModel::Gecko),
        Ok(ProtoEmbeddingModel::GeckoMultilingual) => Ok(EmbeddingModel::GeckoMultilingual),
        Ok(ProtoEmbeddingModel::TextEmbedding004) => Ok(EmbeddingModel::TextEmbedding004),
        Ok(ProtoEmbeddingModel::TextEmbedding005) => Ok(EmbeddingModel::TextEmbedding005),
        Ok(ProtoEmbeddingModel::TextMultilingualEmbedding002) => {
            Ok(EmbeddingModel::TextMultilingualEmbedding002)
        }
        // Cohere models
        Ok(ProtoEmbeddingModel::CohereEmbedV3) => Ok(EmbeddingModel::CohereEmbedV3),
        Ok(ProtoEmbeddingModel::CohereEmbedMultilingualV3) => {
            Ok(EmbeddingModel::CohereEmbedMultilingualV3)
        }
        // Voyage models
        Ok(ProtoEmbeddingModel::Voyage3) => Ok(EmbeddingModel::Voyage3),
        Ok(ProtoEmbeddingModel::Voyage3Lite) => Ok(EmbeddingModel::Voyage3Lite),
        Ok(ProtoEmbeddingModel::VoyageCode3) => Ok(EmbeddingModel::VoyageCode3),
        // Custom
        Ok(ProtoEmbeddingModel::Custom) => {
            let dim = custom_dim.ok_or_else(|| {
                VectorError::Validation(
                    "Custom embedding model requires custom_dimension".to_string(),
                )
            })?;
            Ok(EmbeddingModel::Custom(dim))
        }
        _ => Err(VectorError::Validation(format!(
            "Unknown embedding model: {proto}"
        ))),
    }
}

pub fn embedding_model_to_proto(model: EmbeddingModel) -> i32 {
    match model {
        // OpenAI
        EmbeddingModel::TextEmbedding3Small => ProtoEmbeddingModel::Embedding3Small as i32,
        EmbeddingModel::TextEmbedding3Large => ProtoEmbeddingModel::Embedding3Large as i32,
        EmbeddingModel::TextEmbeddingAda002 => ProtoEmbeddingModel::EmbeddingAda002 as i32,
        // Vertex AI
        EmbeddingModel::Gecko => ProtoEmbeddingModel::Gecko as i32,
        EmbeddingModel::GeckoMultilingual => ProtoEmbeddingModel::GeckoMultilingual as i32,
        EmbeddingModel::TextEmbedding004 => ProtoEmbeddingModel::TextEmbedding004 as i32,
        EmbeddingModel::TextEmbedding005 => ProtoEmbeddingModel::TextEmbedding005 as i32,
        EmbeddingModel::TextMultilingualEmbedding002 => {
            ProtoEmbeddingModel::TextMultilingualEmbedding002 as i32
        }
        // Cohere
        EmbeddingModel::CohereEmbedV3 => ProtoEmbeddingModel::CohereEmbedV3 as i32,
        EmbeddingModel::CohereEmbedMultilingualV3 => {
            ProtoEmbeddingModel::CohereEmbedMultilingualV3 as i32
        }
        // Voyage
        EmbeddingModel::Voyage3 => ProtoEmbeddingModel::Voyage3 as i32,
        EmbeddingModel::Voyage3Lite => ProtoEmbeddingModel::Voyage3Lite as i32,
        EmbeddingModel::VoyageCode3 => ProtoEmbeddingModel::VoyageCode3 as i32,
        // Custom
        EmbeddingModel::Custom(_) => ProtoEmbeddingModel::Custom as i32,
    }
}

// ===== Helper Functions =====

pub fn bytes_to_uuid(bytes: &[u8]) -> VectorResult<Uuid> {
    if bytes.len() != 16 {
        return Err(VectorError::Validation(format!(
            "Invalid UUID: expected 16 bytes, got {}",
            bytes.len()
        )));
    }

    let arr: [u8; 16] = bytes
        .try_into()
        .map_err(|_| VectorError::Validation("Invalid UUID bytes".to_string()))?;

    Ok(Uuid::from_bytes(arr))
}

pub fn uuid_to_bytes(id: Uuid) -> Vec<u8> {
    id.as_bytes().to_vec()
}
