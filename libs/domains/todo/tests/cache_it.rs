//! Rust <-> NATS KV cache integration test.
//!
//! Wraps a counting in-memory repository with `CachedTodoRepository` backed by a
//! real NATS KV bucket (testcontainers) and asserts:
//! - repeated `get_by_id` / `list` are served from KV (inner DB hit once),
//! - writes invalidate the list cache and refresh the id cache,
//! - passthrough mode (no KV) always hits the inner repo.
//!
//! The NATS-backed tests require Docker. Run: `cargo test -p domain_todo --test cache_it`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::Mutex;
use uuid::Uuid;

use domain_todo::models::{CreateTodo, Todo, TodoFilter, TodoPriority, UpdateTodo};
use domain_todo::{open_cache_bucket, CachedTodoRepository, TodoError, TodoRepository, TodoResult};
use test_utils::TestNats;

/// In-memory repository that counts how many reads reach "the DB".
#[derive(Clone)]
struct CountingRepo {
    data: Arc<Mutex<Vec<Todo>>>,
    get_calls: Arc<AtomicUsize>,
    list_calls: Arc<AtomicUsize>,
}

impl CountingRepo {
    fn new(seed: Vec<Todo>) -> Self {
        Self {
            data: Arc::new(Mutex::new(seed)),
            get_calls: Arc::new(AtomicUsize::new(0)),
            list_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn gets(&self) -> usize {
        self.get_calls.load(Ordering::SeqCst)
    }
    fn lists(&self) -> usize {
        self.list_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TodoRepository for CountingRepo {
    async fn create(&self, input: CreateTodo) -> TodoResult<Todo> {
        let now = Utc::now();
        let todo = Todo {
            id: Uuid::now_v7(),
            title: input.title,
            description: input.description,
            completed: false,
            priority: input.priority,
            created_at: now,
            updated_at: now,
        };
        self.data.lock().push(todo.clone());
        Ok(todo)
    }

    async fn get_by_id(&self, id: Uuid) -> TodoResult<Option<Todo>> {
        self.get_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.data.lock().iter().find(|t| t.id == id).cloned())
    }

    async fn list(&self, filter: TodoFilter) -> TodoResult<Vec<Todo>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        let data = self.data.lock();
        let filtered = data
            .iter()
            .filter(|t| filter.completed.is_none_or(|c| t.completed == c))
            .filter(|t| filter.priority.is_none_or(|p| t.priority == p))
            .cloned()
            .skip(filter.offset)
            .take(filter.limit)
            .collect();
        Ok(filtered)
    }

    async fn update(&self, id: Uuid, input: UpdateTodo) -> TodoResult<Todo> {
        let mut data = self.data.lock();
        let todo = data
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or(TodoError::NotFound(id))?;
        todo.apply_update(input);
        Ok(todo.clone())
    }

    async fn delete(&self, id: Uuid) -> TodoResult<bool> {
        let mut data = self.data.lock();
        let before = data.len();
        data.retain(|t| t.id != id);
        Ok(data.len() < before)
    }

    async fn count(&self) -> TodoResult<usize> {
        Ok(self.data.lock().len())
    }
}

fn sample(title: &str, completed: bool) -> Todo {
    let now = Utc::now();
    Todo {
        id: Uuid::now_v7(),
        title: title.to_string(),
        description: String::new(),
        completed,
        priority: TodoPriority::Medium,
        created_at: now,
        updated_at: now,
    }
}

fn all_filter() -> TodoFilter {
    TodoFilter {
        completed: None,
        priority: None,
        limit: 50,
        offset: 0,
    }
}

#[tokio::test]
async fn kv_cache_serves_reads_and_invalidates_on_write() {
    let nats = TestNats::new().await;
    let kv = open_cache_bucket(
        &nats.jetstream(),
        "TODO_CACHE_TEST",
        Duration::from_secs(60),
    )
    .await
    .expect("open KV bucket");

    let t1 = sample("alpha", false);
    let t2 = sample("beta", true);
    let inner = CountingRepo::new(vec![t1.clone(), t2.clone()]);
    let observer = inner.clone();
    let repo = CachedTodoRepository::with_kv(inner, Some(kv));

    // get_by_id: miss (DB) then hit (KV)
    assert_eq!(repo.get_by_id(t1.id).await.unwrap().unwrap().title, "alpha");
    assert_eq!(repo.get_by_id(t1.id).await.unwrap().unwrap().title, "alpha");
    assert_eq!(
        observer.gets(),
        1,
        "second get_by_id must be served from KV"
    );

    // list: miss then hit
    assert_eq!(repo.list(all_filter()).await.unwrap().len(), 2);
    assert_eq!(repo.list(all_filter()).await.unwrap().len(), 2);
    assert_eq!(observer.lists(), 1, "second list must be served from KV");

    // create invalidates the list cache
    repo.create(CreateTodo {
        title: "gamma".to_string(),
        description: String::new(),
        priority: TodoPriority::High,
    })
    .await
    .unwrap();
    assert_eq!(
        repo.list(all_filter()).await.unwrap().len(),
        3,
        "new todo visible after write invalidation"
    );
    assert_eq!(observer.lists(), 2, "list re-fetched from DB after write");

    // update refreshes the id cache (no extra DB read) and reflects new state
    let updated = repo
        .update(
            t1.id,
            UpdateTodo {
                completed: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(updated.completed);
    let got = repo.get_by_id(t1.id).await.unwrap().unwrap();
    assert!(got.completed, "id cache refreshed by update");
    assert_eq!(
        observer.gets(),
        1,
        "get_by_id still served from KV after update refresh"
    );
}

#[tokio::test]
async fn passthrough_when_no_kv_always_hits_inner() {
    let inner = CountingRepo::new(vec![sample("x", false)]);
    let observer = inner.clone();
    let id = observer.data.lock()[0].id;
    let repo = CachedTodoRepository::passthrough(inner);

    repo.get_by_id(id).await.unwrap();
    repo.get_by_id(id).await.unwrap();
    assert_eq!(
        observer.gets(),
        2,
        "passthrough must always hit the inner repo"
    );
}
