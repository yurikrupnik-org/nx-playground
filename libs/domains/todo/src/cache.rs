//! NATS JetStream KV read-cache for the Todo domain.
//!
//! [`CachedTodoRepository`] decorates any [`TodoRepository`] with a NATS KV
//! cache for DB reads:
//! - `get_by_id` is read-through (`id.<uuid>`).
//! - unfiltered `list` (the frontend's hot path) caches the full ordered set
//!   under `list.all` and slices `limit`/`offset` in memory; filtered queries
//!   bypass the cache.
//! - writes (`create`/`update`/`delete`) refresh the id key and drop `list.all`.
//!
//! The cache is **best-effort**: any KV error degrades to a DB hit and is never
//! surfaced to the caller (a cache must not take the read path down). When no KV
//! store is configured (NATS unavailable) the decorator is a transparent
//! pass-through.

use std::time::Duration;

use async_nats::jetstream::kv::{Config, Store};
use async_nats::jetstream::Context;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::error::{TodoError, TodoResult};
use crate::models::{CreateTodo, Todo, TodoFilter, UpdateTodo};
use crate::repository::TodoRepository;

const LIST_KEY: &str = "list.all";

/// Open (or create) the KV bucket used for the DB read-cache.
pub async fn open_cache_bucket(
    jetstream: &Context,
    bucket: &str,
    ttl: Duration,
) -> TodoResult<Store> {
    if let Ok(store) = jetstream.get_key_value(bucket).await {
        return Ok(store);
    }
    jetstream
        .create_key_value(Config {
            bucket: bucket.to_string(),
            description: "Todo DB read cache".to_string(),
            history: 1,
            max_age: ttl,
            ..Default::default()
        })
        .await
        .map_err(|e| TodoError::Internal(format!("open KV bucket {bucket}: {e}")))
}

/// A [`TodoRepository`] wrapped with a NATS KV read-cache.
pub struct CachedTodoRepository<R: TodoRepository> {
    inner: R,
    kv: Option<Store>,
}

impl<R: TodoRepository> CachedTodoRepository<R> {
    /// Wrap `inner` with an optional KV store. `None` => transparent passthrough.
    pub fn with_kv(inner: R, kv: Option<Store>) -> Self {
        Self { inner, kv }
    }

    /// Convenience constructor for a cache-disabled repository.
    pub fn passthrough(inner: R) -> Self {
        Self { inner, kv: None }
    }

    fn id_key(id: Uuid) -> String {
        format!("id.{}", id.simple())
    }

    async fn cache_get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let kv = self.kv.as_ref()?;
        match kv.get(key).await {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes).ok(),
            Ok(None) => None,
            Err(e) => {
                debug!(key, error = %e, "kv get failed; treating as miss");
                None
            }
        }
    }

    async fn cache_put<T: Serialize>(&self, key: &str, value: &T) {
        let Some(kv) = self.kv.as_ref() else {
            return;
        };
        match serde_json::to_vec(value) {
            Ok(bytes) => {
                if let Err(e) = kv.put(key.to_string(), bytes.into()).await {
                    warn!(key, error = %e, "kv put failed");
                }
            }
            Err(e) => warn!(key, error = %e, "kv serialize failed"),
        }
    }

    async fn cache_del(&self, key: &str) {
        let Some(kv) = self.kv.as_ref() else {
            return;
        };
        if let Err(e) = kv.delete(key.to_string()).await {
            warn!(key, error = %e, "kv delete failed");
        }
    }
}

#[async_trait]
impl<R: TodoRepository> TodoRepository for CachedTodoRepository<R> {
    async fn create(&self, input: CreateTodo) -> TodoResult<Todo> {
        let todo = self.inner.create(input).await?;
        self.cache_put(&Self::id_key(todo.id), &todo).await;
        self.cache_del(LIST_KEY).await;
        Ok(todo)
    }

    async fn get_by_id(&self, id: Uuid) -> TodoResult<Option<Todo>> {
        let key = Self::id_key(id);
        if let Some(todo) = self.cache_get::<Todo>(&key).await {
            return Ok(Some(todo));
        }
        let fetched = self.inner.get_by_id(id).await?;
        if let Some(todo) = &fetched {
            self.cache_put(&key, todo).await;
        }
        Ok(fetched)
    }

    async fn list(&self, filter: TodoFilter) -> TodoResult<Vec<Todo>> {
        // Filtered queries bypass the cache (the cached set is the full list).
        if filter.completed.is_some() || filter.priority.is_some() {
            return self.inner.list(filter).await;
        }

        let all: Vec<Todo> = match self.cache_get::<Vec<Todo>>(LIST_KEY).await {
            Some(cached) => cached,
            None => {
                // Fetch the complete ordered set, cache it, then slice.
                let full = self
                    .inner
                    .list(TodoFilter {
                        completed: None,
                        priority: None,
                        // i64::MAX renders as a valid Postgres BIGINT limit ("all").
                        limit: i64::MAX as usize,
                        offset: 0,
                    })
                    .await?;
                self.cache_put(LIST_KEY, &full).await;
                full
            }
        };

        Ok(all
            .into_iter()
            .skip(filter.offset)
            .take(filter.limit)
            .collect())
    }

    async fn update(&self, id: Uuid, input: UpdateTodo) -> TodoResult<Todo> {
        let todo = self.inner.update(id, input).await?;
        self.cache_put(&Self::id_key(id), &todo).await;
        self.cache_del(LIST_KEY).await;
        Ok(todo)
    }

    async fn delete(&self, id: Uuid) -> TodoResult<bool> {
        let deleted = self.inner.delete(id).await?;
        if deleted {
            self.cache_del(&Self::id_key(id)).await;
            self.cache_del(LIST_KEY).await;
        }
        Ok(deleted)
    }

    async fn count(&self) -> TodoResult<usize> {
        // Counts are cheap and change on every write; not cached.
        self.inner.count().await
    }
}
