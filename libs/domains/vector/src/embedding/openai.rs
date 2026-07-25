#![allow(clippy::result_large_err)]

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::EmbeddingProvider;
use crate::error::{VectorError, VectorResult};
use crate::models::{EmbeddingModel, EmbeddingProviderType, EmbeddingResult};

/// OpenAI embedding provider configuration
#[derive(Debug, Clone)]
pub struct OpenAIConfig {
    pub api_key: String,
    pub base_url: String,
}

impl OpenAIConfig {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    pub fn from_env() -> VectorResult<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| VectorError::Config("OPENAI_API_KEY not set".to_string()))?;

        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

        Ok(Self { api_key, base_url })
    }
}

/// OpenAI embeddings provider
pub struct OpenAIProvider {
    client: Client,
    config: OpenAIConfig,
}

impl OpenAIProvider {
    pub fn new(config: OpenAIConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    pub fn from_env() -> VectorResult<Self> {
        Ok(Self::new(OpenAIConfig::from_env()?))
    }
}

/// Request body for the embeddings endpoint; borrows the caller's texts to
/// avoid copying every document per request.
#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'static str,
    input: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
    usage: EmbeddingUsage,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct EmbeddingUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}

#[async_trait]
impl EmbeddingProvider for OpenAIProvider {
    fn provider_type(&self) -> EmbeddingProviderType {
        EmbeddingProviderType::OpenAI
    }

    async fn embed(&self, model: EmbeddingModel, text: &str) -> VectorResult<EmbeddingResult> {
        let texts = [text.to_owned()];
        let results = self.embed_batch(model, &texts).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| VectorError::Embedding("No embedding returned".to_string()))
    }

    async fn embed_batch(
        &self,
        model: EmbeddingModel,
        texts: &[String],
    ) -> VectorResult<Vec<EmbeddingResult>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let dimensions = match model {
            EmbeddingModel::Custom(dim) => Some(dim),
            _ => None,
        };

        let request = EmbeddingRequest {
            model: model.model_name(),
            input: texts,
            dimensions,
        };

        let response = self
            .client
            .post(format!("{}/embeddings", self.config.base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(VectorError::Embedding(format!(
                "OpenAI API error ({status}): {error_text}"
            )));
        }

        let embedding_response: EmbeddingResponse = response.json().await?;

        // Sort by index to maintain order
        let mut data = embedding_response.data;
        data.sort_by_key(|d| d.index);

        let tokens_per_embedding = embedding_response.usage.total_tokens / texts.len() as u32;

        Ok(data
            .into_iter()
            .map(|d| EmbeddingResult {
                dimension: d.embedding.len() as u32,
                values: d.embedding,
                tokens_used: tokens_per_embedding,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_names() {
        assert_eq!(
            EmbeddingModel::TextEmbedding3Small.model_name(),
            "text-embedding-3-small"
        );
        assert_eq!(
            EmbeddingModel::TextEmbedding3Large.model_name(),
            "text-embedding-3-large"
        );
        assert_eq!(
            EmbeddingModel::TextEmbeddingAda002.model_name(),
            "text-embedding-ada-002"
        );
    }

    #[test]
    fn test_model_dimensions() {
        assert_eq!(EmbeddingModel::TextEmbedding3Small.dimension(), 1536);
        assert_eq!(EmbeddingModel::TextEmbedding3Large.dimension(), 3072);
        assert_eq!(EmbeddingModel::Custom(768).dimension(), 768);
    }
}
