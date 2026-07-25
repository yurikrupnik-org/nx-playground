//! Vector Domain Library
//!
//! This module provides a complete domain implementation for vector storage and search,
//! wrapping Qdrant with optional embedding generation capabilities.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────┐
//! │  VectorService  │  ← High-level operations, embedding integration
//! └────────┬────────┘
//!          │
//! ┌────────▼────────┐     ┌─────────────────┐
//! │ VectorRepository│     │ EmbeddingProvider│
//! │   (trait)       │     │    (trait)       │
//! └────────┬────────┘     └────────┬────────┘
//!          │                       │
//! ┌────────▼────────┐     ┌────────▼────────┐
//! │ QdrantRepository│     │  OpenAIProvider  │
//! │ (implementation)│     │  VertexAIProvider│
//! └─────────────────┘     │  (+ more...)     │
//!                         └──────────────────┘
//! ```
//!
//! # Features
//!
//! - **Multi-tenancy**: Project-based collection isolation with optional namespaces
//! - **Vector Operations**: Upsert, search, get, delete with batch support
//! - **Embedding Generation**: Multiple provider support (OpenAI, Vertex AI, Cohere, Voyage)
//! - **Recommendations**: Similar item discovery based on positive/negative examples
//!
//! # Usage
//!
//! ```rust,no_run
//! use domain_vector::{
//!     QdrantRepository, QdrantConfig, VectorService,
//!     TenantContext, CreateCollection, VectorConfig, Vector,
//! };
//! use std::sync::Arc;
//! use uuid::Uuid;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create Qdrant repository
//! let config = QdrantConfig::from_env()?;
//! let repository = QdrantRepository::new(config).await?;
//!
//! // Create service
//! let service = VectorService::new(repository);
//!
//! // Define tenant context for multi-tenancy
//! let tenant = TenantContext::new(Uuid::new_v4());
//!
//! // Create a collection
//! let collection_input = CreateCollection {
//!     name: "documents".to_string(),
//!     config: VectorConfig::new(1536),
//! };
//! service.create_collection(&tenant, collection_input).await?;
//!
//! // Upsert a vector
//! let vector = Vector::new(Uuid::new_v4(), vec![0.1; 1536]);
//! service.upsert(&tenant, "documents", vector, true).await?;
//! # Ok(())
//! # }
//! ```

pub mod conversions;
pub mod embedding;
pub mod error;
pub mod handlers;
pub mod models;
pub mod qdrant;
pub mod repository;
pub mod service;

// Re-export commonly used types
pub use embedding::{EmbeddingProvider, OpenAIProvider, VertexAIProvider};
pub use error::{VectorError, VectorResult};
pub use handlers::VectorApiDoc;
pub use models::{
    CollectionInfo, CollectionStatus, CreateCollection, DistanceMetric, EmbeddingModel,
    EmbeddingProviderType, EmbeddingResult, HnswConfig, RecommendQuery, SearchFilter, SearchQuery,
    SearchResult, SparseVector, TenantContext, Vector, VectorConfig,
};
pub use qdrant::{QdrantConfig, QdrantRepository};
pub use repository::VectorRepository;
pub use service::{SearchWithEmbedding, UpsertWithEmbedding, VectorService};
