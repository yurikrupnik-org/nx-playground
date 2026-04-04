//! Core Dapr HTTP client wrapper.

use eyre::Result;

/// HTTP client for the Dapr sidecar.
///
/// The Dapr sidecar runs on `localhost` at a configurable HTTP port (default 3500).
/// All Dapr API calls go through this client.
#[derive(Clone)]
pub struct DaprClient {
    http: reqwest::Client,
    base_url: String,
}

impl DaprClient {
    /// Create a new Dapr client.
    ///
    /// Reads `DAPR_HTTP_PORT` env var (default: 3500) to build the base URL.
    pub fn new() -> Self {
        let port = std::env::var("DAPR_HTTP_PORT").unwrap_or_else(|_| "3500".to_string());
        Self {
            http: reqwest::Client::new(),
            base_url: format!("http://localhost:{}", port),
        }
    }

    /// Create a Dapr client with a custom base URL (useful for testing).
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
        }
    }

    /// GET request to the Dapr sidecar.
    pub async fn get(&self, path: &str) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.get(&url).send().await?;
        Ok(resp)
    }

    /// POST request to the Dapr sidecar with a JSON body.
    pub async fn post<T: serde::Serialize>(&self, path: &str, body: &T) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.post(&url).json(body).send().await?;
        Ok(resp)
    }

    /// DELETE request to the Dapr sidecar.
    pub async fn delete(&self, path: &str) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.delete(&url).send().await?;
        Ok(resp)
    }

    /// Returns the base URL of the Dapr sidecar.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Default for DaprClient {
    fn default() -> Self {
        Self::new()
    }
}
