#![allow(clippy::result_large_err)]

use crate::error::VectorResult;

/// Qdrant connection configuration
#[derive(Debug, Clone)]
pub struct QdrantConfig {
    pub url: String,
    pub api_key: Option<String>,
    pub timeout_secs: u64,
}

impl QdrantConfig {
    pub fn new(url: String) -> Self {
        Self {
            url,
            api_key: None,
            timeout_secs: 30,
        }
    }

    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn from_env() -> VectorResult<Self> {
        let url =
            std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());

        let api_key = std::env::var("QDRANT_API_KEY").ok();

        let timeout_secs = std::env::var("QDRANT_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);

        Ok(Self {
            url,
            api_key,
            timeout_secs,
        })
    }
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:6334".to_string(),
            api_key: None,
            timeout_secs: 30,
        }
    }
}
