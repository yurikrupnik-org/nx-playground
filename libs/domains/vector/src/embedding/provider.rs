#![allow(clippy::result_large_err)]

use async_trait::async_trait;

use crate::error::VectorResult;
use crate::models::{EmbeddingModel, EmbeddingProviderType, EmbeddingResult};

/// Trait for embedding generation providers
///
/// Implementations can use different embedding APIs (OpenAI, Anthropic, local models).
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Get the provider type
    fn provider_type(&self) -> EmbeddingProviderType;

    /// Generate embedding for a single text
    async fn embed(&self, model: EmbeddingModel, text: &str) -> VectorResult<EmbeddingResult>;

    /// Generate embeddings for multiple texts in batch
    async fn embed_batch(
        &self,
        model: EmbeddingModel,
        texts: &[String],
    ) -> VectorResult<Vec<EmbeddingResult>>;
}
