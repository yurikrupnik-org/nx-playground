//! Dapr state store CRUD operations.

use crate::client::DaprClient;
use eyre::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use tracing::{debug, instrument};

/// Client for Dapr state store operations.
#[derive(Clone)]
pub struct StateClient {
    dapr: DaprClient,
    store_name: String,
}

/// A single state item for batch save operations.
#[derive(Serialize)]
struct StateItem<'a, T: Serialize> {
    key: &'a str,
    value: &'a T,
}

impl StateClient {
    /// Create a new state client for the given Dapr state store component.
    pub fn new(dapr: DaprClient, store_name: impl Into<String>) -> Self {
        Self {
            dapr,
            store_name: store_name.into(),
        }
    }

    /// Save a value to the state store.
    ///
    /// Uses `POST /v1.0/state/{storename}`.
    #[instrument(skip(self, value), fields(store = %self.store_name, key))]
    pub async fn save<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let path = format!("/v1.0/state/{}", self.store_name);
        let items = vec![StateItem { key, value }];
        let resp = self
            .dapr
            .post(&path, &items)
            .await
            .wrap_err("Failed to save state")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(eyre::eyre!(
                "Dapr state save failed: status={}, body={}",
                status,
                body
            ));
        }

        debug!(key, "State saved");
        Ok(())
    }

    /// Get a value from the state store.
    ///
    /// Uses `GET /v1.0/state/{storename}/{key}`.
    /// Returns `None` if the key doesn't exist.
    #[instrument(skip(self), fields(store = %self.store_name, key))]
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let path = format!("/v1.0/state/{}/{}", self.store_name, key);
        let resp = self
            .dapr
            .get(&path)
            .await
            .wrap_err("Failed to get state")?;

        if resp.status() == reqwest::StatusCode::NO_CONTENT
            || resp.status() == reqwest::StatusCode::NOT_FOUND
        {
            return Ok(None);
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(eyre::eyre!(
                "Dapr state get failed: status={}, body={}",
                status,
                body
            ));
        }

        let value = resp.json::<T>().await.wrap_err("Failed to deserialize state")?;
        debug!(key, "State retrieved");
        Ok(Some(value))
    }

    /// Delete a value from the state store.
    ///
    /// Uses `DELETE /v1.0/state/{storename}/{key}`.
    #[instrument(skip(self), fields(store = %self.store_name, key))]
    pub async fn delete(&self, key: &str) -> Result<()> {
        let path = format!("/v1.0/state/{}/{}", self.store_name, key);
        let resp = self
            .dapr
            .delete(&path)
            .await
            .wrap_err("Failed to delete state")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(eyre::eyre!(
                "Dapr state delete failed: status={}, body={}",
                status,
                body
            ));
        }

        debug!(key, "State deleted");
        Ok(())
    }

    /// Query the state store (alpha API).
    ///
    /// Uses `POST /v1.0-alpha1/state/{storename}/query`.
    #[instrument(skip(self, query), fields(store = %self.store_name))]
    pub async fn query<T: DeserializeOwned>(&self, query: &serde_json::Value) -> Result<QueryResponse<T>> {
        let path = format!("/v1.0-alpha1/state/{}/query", self.store_name);
        let resp = self
            .dapr
            .post(&path, query)
            .await
            .wrap_err("Failed to query state")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(eyre::eyre!(
                "Dapr state query failed: status={}, body={}",
                status,
                body
            ));
        }

        let result = resp
            .json::<QueryResponse<T>>()
            .await
            .wrap_err("Failed to deserialize query response")?;
        debug!(results = result.results.len(), "State query complete");
        Ok(result)
    }

    /// Returns the store name.
    pub fn store_name(&self) -> &str {
        &self.store_name
    }
}

/// Response from a state store query.
#[derive(Debug, serde::Deserialize)]
pub struct QueryResponse<T> {
    pub results: Vec<QueryResult<T>>,
    pub token: Option<String>,
}

/// A single result from a state store query.
#[derive(Debug, serde::Deserialize)]
pub struct QueryResult<T> {
    pub key: String,
    pub data: T,
    pub etag: Option<String>,
}
