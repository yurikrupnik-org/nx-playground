use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use qdrant_client::qdrant::{
    self, CreateCollectionBuilder, DeletePointsBuilder, Distance, GetPointsBuilder, PointId,
    PointStruct, RecommendPointsBuilder, SearchPointsBuilder, UpsertPointsBuilder,
    Value as QdrantValue, VectorParamsBuilder,
};
use qdrant_client::Qdrant;
use uuid::Uuid;

use super::QdrantConfig;
use crate::error::{VectorError, VectorResult};
use crate::models::{
    CollectionInfo, CollectionStatus, CreateCollection, DistanceMetric, RecommendQuery,
    SearchQuery, SearchResult, TenantContext, Vector, VectorConfig,
};
use crate::repository::VectorRepository;

/// Qdrant-backed implementation of VectorRepository
pub struct QdrantRepository {
    client: Qdrant,
}

impl QdrantRepository {
    pub async fn new(config: QdrantConfig) -> VectorResult<Self> {
        let mut builder = Qdrant::from_url(&config.url);

        if let Some(api_key) = config.api_key {
            builder = builder.api_key(api_key);
        }

        builder = builder.timeout(Duration::from_secs(config.timeout_secs));

        let client = builder
            .build()
            .map_err(|e| VectorError::Qdrant(format!("Failed to build client: {}", e)))?;

        Ok(Self { client })
    }

    pub fn from_client(client: Qdrant) -> Self {
        Self { client }
    }

    fn to_qdrant_distance(metric: DistanceMetric) -> Distance {
        match metric {
            DistanceMetric::Cosine => Distance::Cosine,
            DistanceMetric::Euclidean => Distance::Euclid,
            DistanceMetric::DotProduct => Distance::Dot,
            DistanceMetric::Manhattan => Distance::Manhattan,
        }
    }

    fn from_qdrant_distance(distance: Distance) -> DistanceMetric {
        match distance {
            Distance::Cosine => DistanceMetric::Cosine,
            Distance::Euclid => DistanceMetric::Euclidean,
            Distance::Dot => DistanceMetric::DotProduct,
            Distance::Manhattan => DistanceMetric::Manhattan,
            _ => DistanceMetric::Cosine,
        }
    }

    fn uuid_to_point_id(id: Uuid) -> PointId {
        PointId::from(id.to_string())
    }

    fn point_id_to_uuid(point_id: &PointId) -> VectorResult<Uuid> {
        match &point_id.point_id_options {
            Some(qdrant::point_id::PointIdOptions::Uuid(uuid_str)) => Uuid::parse_str(uuid_str)
                .map_err(|e| VectorError::Internal(format!("Invalid UUID: {}", e))),
            Some(qdrant::point_id::PointIdOptions::Num(num)) => {
                // If stored as number, create UUID from it
                Ok(Uuid::from_u128(*num as u128))
            }
            None => Err(VectorError::Internal("Missing point ID".to_string())),
        }
    }

    fn payload_to_qdrant(payload: Option<serde_json::Value>) -> HashMap<String, QdrantValue> {
        let Some(value) = payload else {
            return HashMap::new();
        };

        let mut result = HashMap::new();

        if let serde_json::Value::Object(map) = value {
            for (key, val) in map {
                if let Some(qdrant_val) = json_to_qdrant_value(val) {
                    result.insert(key, qdrant_val);
                }
            }
        }

        result
    }

    fn qdrant_to_payload(payload: HashMap<String, QdrantValue>) -> Option<serde_json::Value> {
        if payload.is_empty() {
            return None;
        }

        let mut map = serde_json::Map::new();
        for (key, val) in payload {
            if let Some(json_val) = qdrant_value_to_json(val) {
                map.insert(key, json_val);
            }
        }

        Some(serde_json::Value::Object(map))
    }
}

fn json_to_qdrant_value(val: serde_json::Value) -> Option<QdrantValue> {
    match val {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(b) => Some(QdrantValue::from(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(QdrantValue::from(i))
            } else {
                n.as_f64().map(QdrantValue::from)
            }
        }
        serde_json::Value::String(s) => Some(QdrantValue::from(s)),
        _ => {
            // For complex types, serialize to string
            Some(QdrantValue::from(val.to_string()))
        }
    }
}

fn qdrant_value_to_json(val: QdrantValue) -> Option<serde_json::Value> {
    use qdrant::value::Kind;

    match val.kind {
        Some(Kind::NullValue(_)) => Some(serde_json::Value::Null),
        Some(Kind::BoolValue(b)) => Some(serde_json::Value::Bool(b)),
        Some(Kind::IntegerValue(i)) => Some(serde_json::Value::Number(i.into())),
        Some(Kind::DoubleValue(f)) => {
            serde_json::Number::from_f64(f).map(serde_json::Value::Number)
        }
        Some(Kind::StringValue(s)) => Some(serde_json::Value::String(s)),
        _ => None,
    }
}

#[async_trait]
impl VectorRepository for QdrantRepository {
    async fn create_collection(
        &self,
        tenant: &TenantContext,
        input: CreateCollection,
    ) -> VectorResult<CollectionInfo> {
        let full_name = tenant.collection_name(&input.name);

        let mut builder =
            CreateCollectionBuilder::new(&full_name).vectors_config(VectorParamsBuilder::new(
                input.config.dimension as u64,
                Self::to_qdrant_distance(input.config.distance),
            ));

        if let Some(hnsw) = &input.config.hnsw {
            let mut hnsw_config = qdrant::HnswConfigDiff::default();
            if let Some(m) = hnsw.m {
                hnsw_config.m = Some(m as u64);
            }
            if let Some(ef) = hnsw.ef_construct {
                hnsw_config.ef_construct = Some(ef as u64);
            }
            if let Some(threshold) = hnsw.full_scan_threshold {
                hnsw_config.full_scan_threshold = Some(threshold as u64);
            }
            builder = builder.hnsw_config(hnsw_config);
        }

        self.client.create_collection(builder).await?;

        Ok(CollectionInfo {
            name: full_name,
            vectors_count: 0,
            indexed_vectors_count: 0,
            points_count: 0,
            config: input.config,
            status: CollectionStatus::Green,
        })
    }

    async fn delete_collection(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
    ) -> VectorResult<bool> {
        let full_name = tenant.collection_name(collection_name);
        self.client.delete_collection(&full_name).await?;
        Ok(true)
    }

    async fn get_collection(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
    ) -> VectorResult<Option<CollectionInfo>> {
        let full_name = tenant.collection_name(collection_name);
        self.get_collection_by_full_name(&full_name).await
    }

    async fn list_collections(&self, tenant: &TenantContext) -> VectorResult<Vec<CollectionInfo>> {
        let prefix = format!("{}_", tenant.project_id);
        let collections = self.client.list_collections().await?;

        let mut results = Vec::new();
        for collection in collections.collections {
            if collection.name.starts_with(&prefix) {
                if let Some(info) = self.get_collection_by_full_name(&collection.name).await? {
                    results.push(info);
                }
            }
        }

        Ok(results)
    }

    async fn upsert(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        vector: Vector,
        wait: bool,
    ) -> VectorResult<Uuid> {
        let full_name = tenant.collection_name(collection_name);

        let point = PointStruct::new(
            Self::uuid_to_point_id(vector.id),
            vector.values,
            Self::payload_to_qdrant(vector.payload),
        );

        let mut builder = UpsertPointsBuilder::new(&full_name, vec![point]);
        if wait {
            builder = builder.wait(true);
        }

        self.client.upsert_points(builder).await?;

        Ok(vector.id)
    }

    async fn upsert_batch(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        vectors: Vec<Vector>,
        wait: bool,
    ) -> VectorResult<Vec<Uuid>> {
        let full_name = tenant.collection_name(collection_name);

        let ids: Vec<Uuid> = vectors.iter().map(|v| v.id).collect();

        let points: Vec<PointStruct> = vectors
            .into_iter()
            .map(|v| {
                PointStruct::new(
                    Self::uuid_to_point_id(v.id),
                    v.values,
                    Self::payload_to_qdrant(v.payload),
                )
            })
            .collect();

        let mut builder = UpsertPointsBuilder::new(&full_name, points);
        if wait {
            builder = builder.wait(true);
        }

        self.client.upsert_points(builder).await?;

        Ok(ids)
    }

    async fn search(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        query: SearchQuery,
    ) -> VectorResult<Vec<SearchResult>> {
        let full_name = tenant.collection_name(collection_name);

        let mut builder = SearchPointsBuilder::new(&full_name, query.vector, query.limit as u64);

        if let Some(threshold) = query.score_threshold {
            builder = builder.score_threshold(threshold);
        }

        builder = builder.with_vectors(query.with_vectors);
        builder = builder.with_payload(query.with_payloads);

        let results = self.client.search_points(builder).await?;

        results
            .result
            .into_iter()
            .map(|point| {
                let id = point
                    .id
                    .as_ref()
                    .map(Self::point_id_to_uuid)
                    .transpose()?
                    .ok_or_else(|| VectorError::Internal("Missing point ID".to_string()))?;

                let vector = Self::extract_vector_from_output(&point.vectors);

                Ok(SearchResult {
                    id,
                    score: point.score,
                    payload: Self::qdrant_to_payload(point.payload),
                    vector,
                })
            })
            .collect()
    }

    async fn get(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        ids: Vec<Uuid>,
        with_vectors: bool,
        with_payloads: bool,
    ) -> VectorResult<Vec<Vector>> {
        let full_name = tenant.collection_name(collection_name);

        let point_ids: Vec<PointId> = ids.iter().map(|id| Self::uuid_to_point_id(*id)).collect();

        let mut builder = GetPointsBuilder::new(&full_name, point_ids);

        builder = builder.with_vectors(with_vectors);
        builder = builder.with_payload(with_payloads);

        let results = self.client.get_points(builder).await?;

        results
            .result
            .into_iter()
            .map(|point| {
                let id = point
                    .id
                    .as_ref()
                    .map(Self::point_id_to_uuid)
                    .transpose()?
                    .ok_or_else(|| VectorError::Internal("Missing point ID".to_string()))?;

                let values =
                    Self::extract_vector_from_output(&point.vectors).unwrap_or_else(|| {
                        tracing::warn!("Missing vector data for point {id}, returning empty");
                        Vec::new()
                    });

                Ok(Vector {
                    id,
                    values,
                    payload: Self::qdrant_to_payload(point.payload),
                    sparse: None,
                })
            })
            .collect()
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        ids: Vec<Uuid>,
        wait: bool,
    ) -> VectorResult<u32> {
        let full_name = tenant.collection_name(collection_name);

        let point_ids: Vec<PointId> = ids.iter().map(|id| Self::uuid_to_point_id(*id)).collect();
        let count = point_ids.len() as u32;

        let mut builder = DeletePointsBuilder::new(&full_name).points(point_ids);

        if wait {
            builder = builder.wait(true);
        }

        self.client.delete_points(builder).await?;

        Ok(count)
    }

    async fn recommend(
        &self,
        tenant: &TenantContext,
        collection_name: &str,
        query: RecommendQuery,
    ) -> VectorResult<Vec<SearchResult>> {
        let full_name = tenant.collection_name(collection_name);

        let positive: Vec<PointId> = query
            .positive_ids
            .iter()
            .map(|id| Self::uuid_to_point_id(*id))
            .collect();

        let negative: Vec<PointId> = query
            .negative_ids
            .iter()
            .map(|id| Self::uuid_to_point_id(*id))
            .collect();

        let mut builder = RecommendPointsBuilder::new(&full_name, query.limit as u64);

        // Add each positive example individually
        for point_id in positive {
            builder = builder.add_positive(point_id);
        }

        // Add each negative example individually
        for point_id in negative {
            builder = builder.add_negative(point_id);
        }

        if let Some(threshold) = query.score_threshold {
            builder = builder.score_threshold(threshold);
        }

        builder = builder.with_vectors(query.with_vectors);
        builder = builder.with_payload(query.with_payloads);

        let results = self.client.recommend(builder).await?;

        results
            .result
            .into_iter()
            .map(|point| {
                let id = point
                    .id
                    .as_ref()
                    .map(Self::point_id_to_uuid)
                    .transpose()?
                    .ok_or_else(|| VectorError::Internal("Missing point ID".to_string()))?;

                let vector = Self::extract_vector_from_output(&point.vectors);

                Ok(SearchResult {
                    id,
                    score: point.score,
                    payload: Self::qdrant_to_payload(point.payload),
                    vector,
                })
            })
            .collect()
    }
}

impl QdrantRepository {
    /// Extract vector values from VectorsOutput
    /// Note: Uses deprecated data field for now until migration to 1.18+
    #[allow(deprecated)]
    fn extract_vector_from_output(vectors: &Option<qdrant::VectorsOutput>) -> Option<Vec<f32>> {
        match vectors {
            Some(qdrant::VectorsOutput {
                vectors_options: Some(opts),
            }) => {
                match opts {
                    qdrant::vectors_output::VectorsOptions::Vector(v) => Some(v.data.clone()),
                    qdrant::vectors_output::VectorsOptions::Vectors(map) => {
                        // For multi-vector, return the first one
                        map.vectors.values().next().map(|v| v.data.clone())
                    }
                }
            }
            _ => None,
        }
    }

    async fn get_collection_by_full_name(
        &self,
        full_name: &str,
    ) -> VectorResult<Option<CollectionInfo>> {
        let info = match self.client.collection_info(full_name).await {
            Ok(info) => info,
            Err(_) => return Ok(None),
        };

        let result = info
            .result
            .ok_or_else(|| VectorError::Internal("Collection info missing result".to_string()))?;

        // Extract dimension and distance from config
        let (dimension, distance) = Self::extract_config_params(&result.config);

        let status = match result.status() {
            qdrant::CollectionStatus::Green => CollectionStatus::Green,
            qdrant::CollectionStatus::Yellow => CollectionStatus::Yellow,
            _ => CollectionStatus::Grey,
        };

        // Get counts from counters if available
        let (vectors_count, points_count) = Self::extract_counts(&result);

        Ok(Some(CollectionInfo {
            name: full_name.to_string(),
            vectors_count,
            indexed_vectors_count: vectors_count,
            points_count,
            config: VectorConfig {
                dimension,
                distance,
                hnsw: None,
            },
            status,
        }))
    }

    fn extract_config_params(config: &Option<qdrant::CollectionConfig>) -> (u32, DistanceMetric) {
        match config {
            Some(config) => {
                match &config.params {
                    Some(params) => {
                        match &params.vectors_config {
                            Some(vc) => {
                                match &vc.config {
                                    Some(qdrant::vectors_config::Config::Params(p)) => {
                                        (p.size as u32, Self::from_qdrant_distance(p.distance()))
                                    }
                                    Some(qdrant::vectors_config::Config::ParamsMap(map)) => {
                                        // For multi-vector collections, get first vector config
                                        if let Some((_, p)) = map.map.iter().next() {
                                            (
                                                p.size as u32,
                                                Self::from_qdrant_distance(p.distance()),
                                            )
                                        } else {
                                            (0, DistanceMetric::Cosine)
                                        }
                                    }
                                    None => (0, DistanceMetric::Cosine),
                                }
                            }
                            None => (0, DistanceMetric::Cosine),
                        }
                    }
                    None => (0, DistanceMetric::Cosine),
                }
            }
            None => (0, DistanceMetric::Cosine),
        }
    }

    fn extract_counts(result: &qdrant::CollectionInfo) -> (u64, u64) {
        // Try to get counts from segments_count as a proxy
        let segments = result.segments_count;
        (segments, segments)
    }
}
