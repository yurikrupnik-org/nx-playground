//! Server-side JSON client for todo-api — the Rust twin of the Astro
//! variant's `src/lib/api.ts`. Never shipped to the browser.
//!
//! The DTOs mirror only the fields this frontend renders; serde ignores the
//! rest. Wire shape is owned by `domain_todo::models` (`Todo`, `CreateTodo`,
//! `TodoPriority` — serde lowercase) — keep in sync.

use eyre::{eyre, Result, WrapErr};
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};

/// Priorities in form/render order (mirrors `PRIORITIES` in fragments.ts).
pub const PRIORITIES: [Priority; 3] = [Priority::Low, Priority::Medium, Priority::High];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Medium,
    High,
}

impl Priority {
    /// Wire/CSS token: `badge--{low,medium,high}`, form option values.
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
        }
    }

    /// Parse a form value; `None` for anything not in [`PRIORITIES`].
    pub fn parse(value: &str) -> Option<Self> {
        PRIORITIES.into_iter().find(|p| p.as_str() == value)
    }
}

/// The slice of a todo this frontend renders.
#[derive(Clone, Debug, Deserialize)]
pub struct Todo {
    pub id: String,
    pub title: String,
    pub completed: bool,
    pub priority: Priority,
}

#[derive(Debug, Serialize)]
pub struct CreateTodo {
    pub title: String,
    pub description: String,
    pub priority: Priority,
}

/// JSON client bound to one todo-api origin. Cheap to clone (reqwest `Client`
/// is an `Arc` internally).
#[derive(Clone)]
pub struct TodoApi {
    http: Client,
    origin: String,
}

impl TodoApi {
    pub fn new(origin: String) -> Self {
        Self {
            http: Client::new(),
            origin: origin.trim_end_matches('/').to_owned(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/todos{path}", self.origin)
    }

    /// Mirror of api.ts `expectOk`: non-2xx becomes an error carrying status + body.
    async fn expect_ok(response: Response) -> Result<Response> {
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(eyre!("todo-api {status}: {body}"));
        }
        Ok(response)
    }

    pub async fn list(&self) -> Result<Vec<Todo>> {
        let response = self
            .http
            .get(self.url("?limit=100000"))
            .send()
            .await
            .wrap_err("todo-api unreachable")?;
        Ok(Self::expect_ok(response).await?.json().await?)
    }

    /// `None` when the API answers 404 (unknown id — including non-UUID ids).
    pub async fn get(&self, id: &str) -> Result<Option<Todo>> {
        let response = self
            .http
            .get(self.url(&format!("/{id}")))
            .send()
            .await
            .wrap_err("todo-api unreachable")?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(Self::expect_ok(response).await?.json().await?))
    }

    pub async fn create(&self, input: &CreateTodo) -> Result<Todo> {
        let response = self
            .http
            .post(self.url(""))
            .json(input)
            .send()
            .await
            .wrap_err("todo-api unreachable")?;
        Ok(Self::expect_ok(response).await?.json().await?)
    }

    /// Flip completion given the currently rendered state (idempotent per state).
    pub async fn toggle(&self, id: &str, completed: bool) -> Result<Todo> {
        let action = if completed { "uncomplete" } else { "complete" };
        let response = self
            .http
            .post(self.url(&format!("/{id}/{action}")))
            .send()
            .await
            .wrap_err("todo-api unreachable")?;
        Ok(Self::expect_ok(response).await?.json().await?)
    }

    pub async fn remove(&self, id: &str) -> Result<()> {
        let response = self
            .http
            .delete(self.url(&format!("/{id}")))
            .send()
            .await
            .wrap_err("todo-api unreachable")?;
        Self::expect_ok(response).await?;
        Ok(())
    }
}
