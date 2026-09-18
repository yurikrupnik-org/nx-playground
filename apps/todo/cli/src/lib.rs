//! Todo CLI core: a NATS JetStream KV-backed todo store, event-stream
//! management, and status reporting.
//!
//! Todos are stored as JSON [`Todo`] snapshots in the `TODO_CLI` KV bucket
//! (key = todo uuid). Lifecycle changes are published as `todos.>` events via
//! [`domain_todo::NatsTodoPublisher`], so the existing `todo_worker` consumes
//! CLI-driven changes exactly like API-driven ones. The `events` helpers
//! browse the `TODOS` stream and manage its `TODOS_DLQ` dead-letter queue.

use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::Context;
use async_nats::jetstream::consumer::AckPolicy;
use async_nats::jetstream::consumer::pull::Config as PullConfig;
use async_nats::jetstream::kv::{Config, Store};
use chrono::Utc;
use domain_todo::{CreateTodo, Todo, TodoEvent, TodoEventKind, TodoNatsStream, TodoPriority};
use eyre::{Result, bail, eyre};
use futures::{StreamExt, TryStreamExt};
use messaging::nats::{DlqEntry, DlqManager, Redriven, StreamConfig};
use std::collections::VecDeque;
use uuid::Uuid;

/// KV bucket holding the CLI's todos.
pub const BUCKET: &str = "TODO_CLI";

/// Todo store backed by a NATS JetStream KV bucket.
pub struct TodoStore {
    kv: Store,
}

impl TodoStore {
    /// Open the `TODO_CLI` bucket, creating it if absent (idempotent).
    pub async fn open(jetstream: &Context) -> Result<Self> {
        if let Ok(kv) = jetstream.get_key_value(BUCKET).await {
            return Ok(Self { kv });
        }
        let kv = jetstream
            .create_key_value(Config {
                bucket: BUCKET.to_string(),
                description: "Todo CLI store".to_string(),
                history: 1,
                ..Default::default()
            })
            .await
            .map_err(|e| eyre!("open KV bucket {BUCKET}: {e}"))?;
        Ok(Self { kv })
    }

    /// Create and persist a new todo.
    pub async fn add(&self, create: CreateTodo) -> Result<Todo> {
        if create.title.is_empty() || create.title.len() > 255 {
            bail!("title must be 1..=255 characters");
        }
        let now = Utc::now();
        let todo = Todo {
            id: Uuid::now_v7(),
            title: create.title,
            description: create.description,
            completed: false,
            priority: create.priority,
            created_at: now,
            updated_at: now,
        };
        self.put(&todo).await?;
        Ok(todo)
    }

    /// Persist a todo snapshot (insert or overwrite).
    pub async fn put(&self, todo: &Todo) -> Result<()> {
        let payload = serde_json::to_vec(todo)?;
        self.kv
            .put(todo.id.to_string(), payload.into())
            .await
            .map_err(|e| eyre!("put todo {}: {e}", todo.id))?;
        Ok(())
    }

    /// All todos, oldest first.
    pub async fn list(&self) -> Result<Vec<Todo>> {
        let mut keys = self
            .kv
            .keys()
            .await
            .map_err(|e| eyre!("list keys in {BUCKET}: {e}"))?;
        let mut todos = Vec::new();
        while let Some(key) = keys
            .try_next()
            .await
            .map_err(|e| eyre!("read key stream: {e}"))?
        {
            // A key deleted between keys() and get() is not an error.
            if let Some(bytes) = self
                .kv
                .get(&key)
                .await
                .map_err(|e| eyre!("get {key}: {e}"))?
            {
                todos.push(serde_json::from_slice::<Todo>(&bytes)?);
            }
        }
        todos.sort_by_key(|t| t.created_at);
        Ok(todos)
    }

    /// Resolve a todo by id fragment: full uuid, prefix, or the short id
    /// shown by `list` (last 8 chars — uuid v7 *leading* hex is a millisecond
    /// timestamp shared by ids minted within the same ~65s, so short ids come
    /// from the random tail).
    pub async fn resolve(&self, fragment: &str) -> Result<Todo> {
        if fragment.is_empty() {
            bail!("empty id");
        }
        let mut matches: Vec<Todo> = self
            .list()
            .await?
            .into_iter()
            .filter(|t| {
                let id = t.id.to_string();
                id.starts_with(fragment) || id.ends_with(fragment)
            })
            .collect();
        match matches.len() {
            0 => bail!("no todo matches id '{fragment}'"),
            1 => Ok(matches.remove(0)),
            n => bail!("id '{fragment}' is ambiguous ({n} matches); use more characters"),
        }
    }

    /// Remove a todo (purges KV history for the key).
    pub async fn remove(&self, id: Uuid) -> Result<()> {
        self.kv
            .purge(id.to_string())
            .await
            .map_err(|e| eyre!("remove todo {id}: {e}"))?;
        Ok(())
    }
}

/// Snapshot of store and event-stream state for `todo_cli status`.
pub struct StatusReport {
    pub total: usize,
    pub completed: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    /// Messages in the `TODOS` event stream; `None` when no events were ever
    /// published (stream not created yet).
    pub stream_messages: Option<u64>,
    /// Messages in the `TODOS_DLQ` dead-letter stream; `None` when absent
    /// (no worker ever gave up on an event).
    pub dlq_messages: Option<u64>,
}

impl StatusReport {
    pub fn pending(&self) -> usize {
        self.total - self.completed
    }
}

/// Gather counts from the KV store and the `TODOS` JetStream stream.
pub async fn status(jetstream: &Context, store: &TodoStore) -> Result<StatusReport> {
    let todos = store.list().await?;
    let mut report = StatusReport {
        total: todos.len(),
        completed: 0,
        high: 0,
        medium: 0,
        low: 0,
        stream_messages: None,
        dlq_messages: None,
    };
    for todo in &todos {
        if todo.completed {
            report.completed += 1;
        }
        match todo.priority {
            TodoPriority::High => report.high += 1,
            TodoPriority::Medium => report.medium += 1,
            TodoPriority::Low => report.low += 1,
        }
    }
    // Absent stream just means no events were published yet — not an error.
    if let Ok(mut stream) = jetstream.get_stream(TodoNatsStream::STREAM_NAME).await {
        let info = stream
            .info()
            .await
            .map_err(|e| eyre!("stream info for {}: {e}", TodoNatsStream::STREAM_NAME))?;
        report.stream_messages = Some(info.state.messages);
    }
    if let Ok(mut stream) = jetstream.get_stream(TodoNatsStream::DLQ_STREAM).await {
        let info = stream
            .info()
            .await
            .map_err(|e| eyre!("stream info for {}: {e}", TodoNatsStream::DLQ_STREAM))?;
        report.dlq_messages = Some(info.state.messages);
    }
    Ok(report)
}

/// A [`TodoEvent`] read back from the `TODOS` stream.
pub struct RecordedEvent {
    /// Stream sequence number.
    pub sequence: u64,
    pub event: TodoEvent,
}

/// Browse messages of a stream through a throwaway ephemeral consumer,
/// keeping the last `limit` decoded entries. `AckPolicy::None`: browsing must
/// not track delivery state, and the server reaps the consumer after
/// `inactive_threshold`.
async fn browse<T: serde::de::DeserializeOwned>(
    jetstream: &Context,
    stream_name: &str,
    filter_subject: Option<String>,
    limit: usize,
) -> Result<Vec<(u64, T)>> {
    // Absent stream means nothing was ever published — empty, not an error.
    let Ok(stream) = jetstream.get_stream(stream_name).await else {
        return Ok(Vec::new());
    };
    let consumer = stream
        .create_consumer(PullConfig {
            filter_subject: filter_subject.unwrap_or_default(),
            ack_policy: AckPolicy::None,
            inactive_threshold: Duration::from_secs(30),
            ..Default::default()
        })
        .await
        .map_err(|e| eyre!("create browse consumer on {stream_name}: {e}"))?;

    const BATCH: usize = 100;
    let mut kept: VecDeque<(u64, T)> = VecDeque::with_capacity(limit);
    loop {
        let mut batch = consumer
            .fetch()
            .max_messages(BATCH)
            .expires(Duration::from_secs(5))
            .messages()
            .await
            .map_err(|e| eyre!("fetch from {stream_name}: {e}"))?;
        let mut got = 0usize;
        while let Some(msg) = batch.next().await {
            let msg = msg.map_err(|e| eyre!("read message from {stream_name}: {e}"))?;
            got += 1;
            let sequence = msg
                .info()
                .map_err(|e| eyre!("message info: {e}"))?
                .stream_sequence;
            // Undecodable messages are foreign to this CLI; skip, don't fail the browse.
            if let Ok(decoded) = serde_json::from_slice::<T>(&msg.payload) {
                if kept.len() == limit {
                    kept.pop_front();
                }
                kept.push_back((sequence, decoded));
            }
        }
        if got < BATCH {
            return Ok(kept.into_iter().collect());
        }
    }
}

/// The last `limit` events on the `TODOS` stream, oldest first, optionally
/// filtered to one kind (server-side subject filter).
pub async fn recent_events(
    jetstream: &Context,
    kind: Option<TodoEventKind>,
    limit: usize,
) -> Result<Vec<RecordedEvent>> {
    let filter = kind.map(|k| format!("todos.{}", k.subject_suffix()));
    Ok(
        browse::<TodoEvent>(jetstream, TodoNatsStream::STREAM_NAME, filter, limit)
            .await?
            .into_iter()
            .map(|(sequence, event)| RecordedEvent { sequence, event })
            .collect(),
    )
}

/// The last `limit` entries in `TODOS_DLQ`, oldest first.
pub async fn dlq_entries(jetstream: &Context, limit: usize) -> Result<Vec<(u64, DlqEntry)>> {
    browse::<DlqEntry>(jetstream, TodoNatsStream::DLQ_STREAM, None, limit).await
}

/// Republish DLQ entries to their original `todos.*` subjects (run after
/// deploying a fix; see [`DlqManager::redrive`]). Entries stay in the DLQ as
/// an audit trail.
pub async fn redrive_dlq(jetstream: &Context, from: u64, limit: usize) -> Result<Redriven> {
    let dlq = DlqManager::new(Arc::new(jetstream.clone()), TodoNatsStream::DLQ_STREAM);
    dlq.redrive(from, limit)
        .await
        .map_err(|e| eyre!("redrive {}: {e}", TodoNatsStream::DLQ_STREAM))
}

/// Purge all messages from the `TODOS` stream. Returns the purged count;
/// 0 when the stream does not exist. The DLQ is left untouched.
pub async fn purge_events(jetstream: &Context) -> Result<u64> {
    let Ok(stream) = jetstream.get_stream(TodoNatsStream::STREAM_NAME).await else {
        return Ok(0);
    };
    let response = stream
        .purge()
        .await
        .map_err(|e| eyre!("purge {}: {e}", TodoNatsStream::STREAM_NAME))?;
    Ok(response.purged)
}
