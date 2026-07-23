//! Vector gRPC service implementation
//!
//! This module contains the VectorServiceImpl struct and its gRPC trait implementation.

use std::sync::Arc;

use domain_vector::{
    CreateCollection, RecommendQuery, SearchFilter, SearchQuery, SearchWithEmbedding,
    TenantContext, UpsertWithEmbedding, Vector, VectorRepository, VectorService,
    conversions as conv,
};
use rpc::vector::v1::{
    CreateCollectionRequest, CreateCollectionResponse, DeleteCollectionRequest,
    DeleteCollectionResponse, DeleteRequest, DeleteResponse, EmbedBatchRequest, EmbedBatchResponse,
    EmbedRequest, EmbedResponse, EmbeddingResult as ProtoEmbeddingResult, GetCollectionRequest,
    GetCollectionResponse, GetRequest, GetResponse, ListCollectionsRequest,
    ListCollectionsResponse, RecommendRequest, RecommendResponse, SearchRequest, SearchResponse,
    SearchWithEmbeddingRequest, UpsertBatchRequest, UpsertBatchResponse, UpsertRequest,
    UpsertResponse, UpsertWithEmbeddingRequest,
    vector_service_server::VectorService as VectorServiceTrait,
};
use tonic::{Request, Response, Status};
use tracing::info;

/// gRPC service implementation for vector operations
///
/// Wraps the domain VectorService and handles proto ↔ domain conversions.
pub struct VectorServiceImpl<R: VectorRepository> {
    service: Arc<VectorService<R>>,
}

impl<R: VectorRepository> VectorServiceImpl<R> {
    pub fn new(service: VectorService<R>) -> Self {
        Self {
            service: Arc::new(service),
        }
    }
}

// Helper function to convert bytes to TenantContext
fn parse_tenant(tenant: Option<rpc::vector::v1::TenantContext>) -> Result<TenantContext, Status> {
    conv::tenant_from_proto(tenant)
        .map_err(|e| Status::invalid_argument(format!("Invalid tenant context: {e}")))
}

#[tonic::async_trait]
impl<R> VectorServiceTrait for VectorServiceImpl<R>
where
    R: VectorRepository + 'static,
{
    // ===== Collection Management =====

    async fn create_collection(
        &self,
        request: Request<CreateCollectionRequest>,
    ) -> Result<Response<CreateCollectionResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;
        let config = conv::vector_config_from_proto(req.config)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let input = CreateCollection {
            name: req.collection_name.clone(),
            config,
        };

        let info = self
            .service
            .create_collection(&tenant, input)
            .await
            .map_err(|e| Status::internal(format!("Failed to create collection: {}", e)))?;

        info!(
            collection = %info.name,
            "Created collection"
        );

        Ok(Response::new(CreateCollectionResponse {
            full_collection_name: info.name,
            created: true,
        }))
    }

    async fn delete_collection(
        &self,
        request: Request<DeleteCollectionRequest>,
    ) -> Result<Response<DeleteCollectionResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;

        let deleted = self
            .service
            .delete_collection(&tenant, &req.collection_name)
            .await
            .map_err(|e| Status::internal(format!("Failed to delete collection: {}", e)))?;

        Ok(Response::new(DeleteCollectionResponse { deleted }))
    }

    async fn get_collection(
        &self,
        request: Request<GetCollectionRequest>,
    ) -> Result<Response<GetCollectionResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;

        let info = self
            .service
            .get_collection(&tenant, &req.collection_name)
            .await
            .map_err(|e| Status::internal(format!("Failed to get collection: {}", e)))?
            .ok_or_else(|| Status::not_found("Collection not found"))?;

        Ok(Response::new(GetCollectionResponse {
            info: Some(info.into()),
        }))
    }

    async fn list_collections(
        &self,
        request: Request<ListCollectionsRequest>,
    ) -> Result<Response<ListCollectionsResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;

        let collections = self
            .service
            .list_collections(&tenant)
            .await
            .map_err(|e| Status::internal(format!("Failed to list collections: {}", e)))?;

        Ok(Response::new(ListCollectionsResponse {
            collections: collections.into_iter().map(Into::into).collect(),
        }))
    }

    // ===== Vector Operations =====

    async fn upsert(
        &self,
        request: Request<UpsertRequest>,
    ) -> Result<Response<UpsertResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;
        let vector = conv::vector_from_proto(req.vector)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let id = self
            .service
            .upsert(&tenant, &req.collection_name, vector, req.wait)
            .await
            .map_err(|e| Status::internal(format!("Failed to upsert: {}", e)))?;

        Ok(Response::new(UpsertResponse {
            id: id.as_bytes().to_vec(),
            status: if req.wait {
                "completed".to_string()
            } else {
                "pending".to_string()
            },
        }))
    }

    async fn upsert_batch(
        &self,
        request: Request<UpsertBatchRequest>,
    ) -> Result<Response<UpsertBatchResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;

        let vectors: Vec<Vector> = req
            .vectors
            .into_iter()
            .map(|v| {
                v.try_into().map_err(|e: domain_vector::VectorError| {
                    Status::invalid_argument(e.to_string())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let count = vectors.len() as u32;
        let ids = self
            .service
            .upsert_batch(&tenant, &req.collection_name, vectors, req.wait)
            .await
            .map_err(|e| Status::internal(format!("Failed to upsert batch: {}", e)))?;

        Ok(Response::new(UpsertBatchResponse {
            ids: ids.into_iter().map(|id| id.as_bytes().to_vec()).collect(),
            status: if req.wait {
                "completed".to_string()
            } else {
                "pending".to_string()
            },
            upserted_count: count,
        }))
    }

    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;

        let mut query = SearchQuery::new(req.query_vector, req.limit);
        query.score_threshold = req.score_threshold;
        query.with_vectors = req.with_vectors;
        query.with_payloads = req.with_payloads;

        if let Some(filter) = req.filter {
            query.filter = Some(SearchFilter {
                must_have_id: filter
                    .must_have_id
                    .map(|bytes| conv::bytes_to_uuid(&bytes))
                    .transpose()
                    .map_err(|e| Status::invalid_argument(e.to_string()))?,
                must_match: filter.must_match.and_then(|p| {
                    if p.json.is_empty() {
                        None
                    } else {
                        serde_json::from_slice(&p.json).ok()
                    }
                }),
                namespace_filter: filter.namespace_filter,
            });
        }

        let results = self
            .service
            .search(&tenant, &req.collection_name, query)
            .await
            .map_err(|e| Status::internal(format!("Failed to search: {}", e)))?;

        Ok(Response::new(conv::search_results_to_response(results)))
    }

    async fn get(&self, request: Request<GetRequest>) -> Result<Response<GetResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;

        let ids: Vec<uuid::Uuid> = req
            .ids
            .iter()
            .map(|bytes| conv::bytes_to_uuid(bytes))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let vectors = self
            .service
            .get(
                &tenant,
                &req.collection_name,
                ids,
                req.with_vectors,
                req.with_payloads,
            )
            .await
            .map_err(|e| Status::internal(format!("Failed to get: {}", e)))?;

        Ok(Response::new(GetResponse {
            vectors: vectors.into_iter().map(Into::into).collect(),
        }))
    }

    async fn delete(
        &self,
        request: Request<DeleteRequest>,
    ) -> Result<Response<DeleteResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;

        let ids: Vec<uuid::Uuid> = req
            .ids
            .iter()
            .map(|bytes| conv::bytes_to_uuid(bytes))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let deleted_count = self
            .service
            .delete(&tenant, &req.collection_name, ids, req.wait)
            .await
            .map_err(|e| Status::internal(format!("Failed to delete: {}", e)))?;

        Ok(Response::new(DeleteResponse {
            deleted_count,
            status: if req.wait {
                "completed".to_string()
            } else {
                "pending".to_string()
            },
        }))
    }

    // ===== Embedding Operations =====

    async fn embed(
        &self,
        request: Request<EmbedRequest>,
    ) -> Result<Response<EmbedResponse>, Status> {
        let req = request.into_inner();
        let model = conv::embedding_model_from_proto(req.model, req.custom_dimension)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let result = self
            .service
            .embed(model, &req.text)
            .await
            .map_err(|e| Status::internal(format!("Failed to embed: {}", e)))?;

        Ok(Response::new(EmbedResponse {
            embedding: result.values,
            dimension: result.dimension,
            tokens_used: result.tokens_used,
        }))
    }

    async fn embed_batch(
        &self,
        request: Request<EmbedBatchRequest>,
    ) -> Result<Response<EmbedBatchResponse>, Status> {
        let req = request.into_inner();
        let model = conv::embedding_model_from_proto(req.model, req.custom_dimension)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let results = self
            .service
            .embed_batch(model, &req.texts)
            .await
            .map_err(|e| Status::internal(format!("Failed to embed batch: {}", e)))?;

        let total_tokens: u32 = results.iter().map(|r| r.tokens_used).sum();

        Ok(Response::new(EmbedBatchResponse {
            embeddings: results
                .into_iter()
                .map(|r| ProtoEmbeddingResult {
                    values: r.values,
                    dimension: r.dimension,
                })
                .collect(),
            total_tokens,
        }))
    }

    // ===== Combined Operations =====

    async fn upsert_with_embedding(
        &self,
        request: Request<UpsertWithEmbeddingRequest>,
    ) -> Result<Response<UpsertResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;
        let model = conv::embedding_model_from_proto(req.model, None)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let id =
            conv::bytes_to_uuid(&req.id).map_err(|e| Status::invalid_argument(e.to_string()))?;

        let payload = req.payload.and_then(|p| {
            if p.json.is_empty() {
                None
            } else {
                serde_json::from_slice(&p.json).ok()
            }
        });

        let result_id = self
            .service
            .upsert_with_embedding(
                &tenant,
                &req.collection_name,
                UpsertWithEmbedding {
                    id,
                    text: req.text,
                    payload,
                    model,
                    wait: req.wait,
                },
            )
            .await
            .map_err(|e| Status::internal(format!("Failed to upsert with embedding: {}", e)))?;

        Ok(Response::new(UpsertResponse {
            id: result_id.as_bytes().to_vec(),
            status: if req.wait {
                "completed".to_string()
            } else {
                "pending".to_string()
            },
        }))
    }

    async fn search_with_embedding(
        &self,
        request: Request<SearchWithEmbeddingRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;
        let model = conv::embedding_model_from_proto(req.model, None)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let results = self
            .service
            .search_with_embedding(
                &tenant,
                &req.collection_name,
                SearchWithEmbedding {
                    text: req.text,
                    limit: req.limit,
                    score_threshold: req.score_threshold,
                    with_vectors: req.with_vectors,
                    with_payloads: req.with_payloads,
                    model,
                },
            )
            .await
            .map_err(|e| Status::internal(format!("Failed to search with embedding: {}", e)))?;

        Ok(Response::new(conv::search_results_to_response(results)))
    }

    // ===== Recommendations =====

    async fn recommend(
        &self,
        request: Request<RecommendRequest>,
    ) -> Result<Response<RecommendResponse>, Status> {
        let req = request.into_inner();
        let tenant = parse_tenant(req.tenant)?;

        let positive_ids: Vec<uuid::Uuid> = req
            .positive_ids
            .iter()
            .map(|bytes| conv::bytes_to_uuid(bytes))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let negative_ids: Vec<uuid::Uuid> = req
            .negative_ids
            .iter()
            .map(|bytes| conv::bytes_to_uuid(bytes))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let mut filter = None;
        if let Some(f) = req.filter {
            filter = Some(SearchFilter {
                must_have_id: f
                    .must_have_id
                    .map(|bytes| conv::bytes_to_uuid(&bytes))
                    .transpose()
                    .map_err(|e| Status::invalid_argument(e.to_string()))?,
                must_match: f.must_match.and_then(|p| {
                    if p.json.is_empty() {
                        None
                    } else {
                        serde_json::from_slice(&p.json).ok()
                    }
                }),
                namespace_filter: f.namespace_filter,
            });
        }

        let query = RecommendQuery {
            positive_ids,
            negative_ids,
            limit: req.limit,
            score_threshold: req.score_threshold,
            filter,
            with_vectors: req.with_vectors,
            with_payloads: req.with_payloads,
        };

        let results = self
            .service
            .recommend(&tenant, &req.collection_name, query)
            .await
            .map_err(|e| Status::internal(format!("Failed to recommend: {}", e)))?;

        Ok(Response::new(conv::search_results_to_recommend_response(
            results,
        )))
    }
}
