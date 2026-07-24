#![allow(clippy::result_large_err)]

//! Vertex AI embedding provider implementation
//!
//! Uses Google Cloud's Vertex AI text embedding API.
//! Supports authentication via:
//! - Service account JSON file (GOOGLE_APPLICATION_CREDENTIALS)
//! - Workload Identity (in GKE)
//! - Default application credentials

use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::EmbeddingProvider;
use crate::error::{VectorError, VectorResult};
use crate::models::{EmbeddingModel, EmbeddingProviderType, EmbeddingResult};

/// Vertex AI provider configuration
#[derive(Debug, Clone)]
pub struct VertexAIConfig {
    /// GCP Project ID
    pub project_id: String,
    /// GCP Region (e.g., "us-central1")
    pub location: String,
    /// Access token (obtained from Google Auth)
    /// If not provided, will attempt to use Application Default Credentials
    pub access_token: Option<String>,
}

impl VertexAIConfig {
    pub fn new(project_id: String, location: String) -> Self {
        Self {
            project_id,
            location,
            access_token: None,
        }
    }

    pub fn with_access_token(mut self, token: String) -> Self {
        self.access_token = Some(token);
        self
    }

    pub fn from_env() -> VectorResult<Self> {
        let project_id = std::env::var("GOOGLE_CLOUD_PROJECT")
            .or_else(|_| std::env::var("GCP_PROJECT_ID"))
            .map_err(|_| {
                VectorError::Config("GOOGLE_CLOUD_PROJECT or GCP_PROJECT_ID not set".to_string())
            })?;

        let location =
            std::env::var("VERTEX_AI_LOCATION").unwrap_or_else(|_| "us-central1".to_string());

        let access_token = std::env::var("GOOGLE_ACCESS_TOKEN").ok();

        Ok(Self {
            project_id,
            location,
            access_token,
        })
    }

    /// Get the Vertex AI endpoint URL for the given model
    fn endpoint_url(&self, model: &str) -> String {
        format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:predict",
            self.location, self.project_id, self.location, model
        )
    }
}

/// A metadata-server token together with its expiry deadline.
struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// Vertex AI embeddings provider
pub struct VertexAIProvider {
    client: Client,
    config: VertexAIConfig,
    /// Cached metadata-server access token; refreshed shortly before expiry
    /// instead of being re-fetched on every request.
    cached_token: RwLock<Option<CachedToken>>,
}

impl VertexAIProvider {
    /// Refresh the cached token this long before it actually expires.
    const TOKEN_EXPIRY_MARGIN: Duration = Duration::from_secs(60);
    /// Fallback lifetime when the metadata server omits `expires_in`.
    const TOKEN_FALLBACK_TTL: Duration = Duration::from_secs(300);

    pub fn new(config: VertexAIConfig) -> Self {
        Self {
            client: Client::new(),
            config,
            cached_token: RwLock::new(None),
        }
    }

    pub fn from_env() -> VectorResult<Self> {
        Ok(Self::new(VertexAIConfig::from_env()?))
    }

    /// Get an access token, reusing the cached metadata-server token until
    /// shortly before it expires.
    async fn get_access_token(&self) -> VectorResult<String> {
        // A statically configured token takes precedence.
        if let Some(token) = &self.config.access_token {
            return Ok(token.clone());
        }

        if let Some(cached) = self.cached_token.read().await.as_ref() {
            if Instant::now() < cached.expires_at {
                return Ok(cached.token.clone());
            }
        }

        let mut guard = self.cached_token.write().await;
        // Another task may have refreshed while we waited for the write lock.
        if let Some(cached) = guard.as_ref() {
            if Instant::now() < cached.expires_at {
                return Ok(cached.token.clone());
            }
        }

        let (token, ttl) = self.fetch_metadata_token().await?;
        let expires_at = Instant::now() + ttl.saturating_sub(Self::TOKEN_EXPIRY_MARGIN);
        *guard = Some(CachedToken {
            token: token.clone(),
            expires_at,
        });
        Ok(token)
    }

    /// Fetch an access token and its lifetime from the GCP metadata server
    /// (works in GKE with Workload Identity).
    async fn fetch_metadata_token(&self) -> VectorResult<(String, Duration)> {
        let metadata_url = "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

        let response = self
            .client
            .get(metadata_url)
            .header("Metadata-Flavor", "Google")
            .send()
            .await
            .map_err(|e| {
                VectorError::Config(format!(
                    "Failed to get access token from metadata server: {e}. \
                     Set GOOGLE_ACCESS_TOKEN environment variable for local development."
                ))
            })?;

        if !response.status().is_success() {
            return Err(VectorError::Config(
                "Failed to get access token from metadata server. \
                 Set GOOGLE_ACCESS_TOKEN environment variable for local development."
                    .to_string(),
            ));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: Option<u64>,
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(|e| VectorError::Config(format!("Failed to parse token response: {e}")))?;

        let ttl = token_response
            .expires_in
            .map_or(Self::TOKEN_FALLBACK_TTL, Duration::from_secs);

        Ok((token_response.access_token, ttl))
    }

    /// Map EmbeddingModel to Vertex AI model name
    fn model_name(model: EmbeddingModel) -> &'static str {
        match model {
            EmbeddingModel::Gecko => "textembedding-gecko@003",
            EmbeddingModel::GeckoMultilingual => "textembedding-gecko-multilingual@001",
            EmbeddingModel::TextEmbedding004 => "text-embedding-004",
            EmbeddingModel::TextEmbedding005 => "text-embedding-005",
            EmbeddingModel::TextMultilingualEmbedding002 => "text-multilingual-embedding-002",
            // Default to text-embedding-004 for non-Vertex models
            _ => "text-embedding-004",
        }
    }
}

// Vertex AI request/response types

/// Request body for the predict endpoint; borrows the caller's texts to avoid
/// copying every document per request.
#[derive(Debug, Serialize)]
struct VertexAIRequest<'a> {
    instances: Vec<TextInstance<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<EmbeddingParameters>,
}

#[derive(Debug, Serialize)]
struct TextInstance<'a> {
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct EmbeddingParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    output_dimensionality: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct VertexAIResponse {
    predictions: Vec<EmbeddingPrediction>,
    #[serde(default)]
    metadata: Option<ResponseMetadata>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingPrediction {
    embeddings: EmbeddingValues,
}

#[derive(Debug, Deserialize)]
struct EmbeddingValues {
    values: Vec<f32>,
    #[serde(default)]
    statistics: Option<EmbeddingStatistics>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingStatistics {
    #[serde(default)]
    token_count: u32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ResponseMetadata {
    #[serde(default, rename = "billableCharacterCount")]
    billable_character_count: u64,
}

#[async_trait]
impl EmbeddingProvider for VertexAIProvider {
    fn provider_type(&self) -> EmbeddingProviderType {
        EmbeddingProviderType::VertexAI
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

        let access_token = self.get_access_token().await?;
        let model_name = Self::model_name(model);
        let endpoint = self.config.endpoint_url(model_name);

        let instances: Vec<TextInstance<'_>> = texts
            .iter()
            .map(|text| TextInstance {
                content: text,
                task_type: Some("RETRIEVAL_DOCUMENT"),
                title: None,
            })
            .collect();

        let request = VertexAIRequest {
            instances,
            parameters: None,
        };

        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(VectorError::Embedding(format!(
                "Vertex AI API error ({status}): {error_text}"
            )));
        }

        let embedding_response: VertexAIResponse = response.json().await?;

        Ok(embedding_response
            .predictions
            .into_iter()
            .map(|p| {
                let values = p.embeddings.values;
                let dimension = values.len() as u32;
                let tokens_used = p.embeddings.statistics.map(|s| s.token_count).unwrap_or(0);

                EmbeddingResult {
                    values,
                    dimension,
                    tokens_used,
                }
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
            VertexAIProvider::model_name(EmbeddingModel::Gecko),
            "textembedding-gecko@003"
        );
        assert_eq!(
            VertexAIProvider::model_name(EmbeddingModel::TextEmbedding004),
            "text-embedding-004"
        );
        assert_eq!(
            VertexAIProvider::model_name(EmbeddingModel::TextEmbedding005),
            "text-embedding-005"
        );
    }

    #[test]
    fn test_endpoint_url() {
        let config = VertexAIConfig::new("my-project".to_string(), "us-central1".to_string());
        let expected = "https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/text-embedding-004:predict";
        assert_eq!(config.endpoint_url("text-embedding-004"), expected);
    }
}
