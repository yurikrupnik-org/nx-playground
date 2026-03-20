//! MongoDB implementation of TaskRepository

use async_trait::async_trait;
use mongodb::bson::{self, doc, Document};
use mongodb::Collection;
use uuid::Uuid;

use crate::{
    error::{TaskError, TaskResult},
    models::{CreateTask, Task, TaskFilter, UpdateTask},
    repository::TaskRepository,
};

pub struct MongoTaskRepository {
    collection: Collection<Document>,
}

impl MongoTaskRepository {
    pub fn new(db: mongodb::Database) -> Self {
        Self {
            collection: db.collection("tasks"),
        }
    }
}

fn task_from_doc(doc: Document) -> TaskResult<Task> {
    let id_str: String = doc
        .get_str("_id")
        .map_err(|e| TaskError::Internal(format!("Missing _id: {e}")))?
        .to_string();

    Ok(Task {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| TaskError::Internal(format!("Invalid UUID: {e}")))?,
        title: doc
            .get_str("title")
            .unwrap_or_default()
            .to_string(),
        description: doc
            .get_str("description")
            .unwrap_or_default()
            .to_string(),
        project_id: doc
            .get_str("project_id")
            .ok()
            .and_then(|s| Uuid::parse_str(s).ok()),
        priority: doc
            .get_str("priority")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default(),
        status: doc
            .get_str("status")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default(),
        due_date: doc
            .get_datetime("due_date")
            .ok()
            .map(|dt| {
                chrono::DateTime::from_timestamp_millis(dt.timestamp_millis())
                    .unwrap_or_default()
            }),
        created_at: doc
            .get_datetime("created_at")
            .ok()
            .map(|dt| {
                chrono::DateTime::from_timestamp_millis(dt.timestamp_millis())
                    .unwrap_or_default()
            })
            .unwrap_or_else(chrono::Utc::now),
        updated_at: doc
            .get_datetime("updated_at")
            .ok()
            .map(|dt| {
                chrono::DateTime::from_timestamp_millis(dt.timestamp_millis())
                    .unwrap_or_default()
            })
            .unwrap_or_else(chrono::Utc::now),
    })
}

fn task_to_doc(id: Uuid, input: &CreateTask) -> Document {
    let now = bson::DateTime::now();
    let mut doc = doc! {
        "_id": id.to_string(),
        "title": &input.title,
        "description": &input.description,
        "priority": input.priority.to_string(),
        "status": input.status.to_string(),
        "created_at": now,
        "updated_at": now,
    };

    if let Some(project_id) = input.project_id {
        doc.insert("project_id", project_id.to_string());
    }

    if let Some(due_date) = input.due_date {
        doc.insert(
            "due_date",
            bson::DateTime::from_millis(due_date.timestamp_millis()),
        );
    }

    doc
}

#[async_trait]
impl TaskRepository for MongoTaskRepository {
    async fn create(&self, input: CreateTask) -> TaskResult<Task> {
        let id = Uuid::now_v7();
        let doc = task_to_doc(id, &input);

        self.collection
            .insert_one(doc)
            .await
            .map_err(|e| TaskError::Internal(format!("MongoDB insert error: {e}")))?;

        // Read back the inserted document
        let result = self
            .collection
            .find_one(doc! { "_id": id.to_string() })
            .await
            .map_err(|e| TaskError::Internal(format!("MongoDB find error: {e}")))?
            .ok_or_else(|| TaskError::Internal("Insert succeeded but document not found".into()))?;

        task_from_doc(result)
    }

    async fn get_by_id(&self, id: Uuid) -> TaskResult<Option<Task>> {
        let result = self
            .collection
            .find_one(doc! { "_id": id.to_string() })
            .await
            .map_err(|e| TaskError::Internal(format!("MongoDB find error: {e}")))?;

        match result {
            Some(doc) => Ok(Some(task_from_doc(doc)?)),
            None => Ok(None),
        }
    }

    async fn list(&self, filter: TaskFilter) -> TaskResult<Vec<Task>> {
        use futures::TryStreamExt;

        let mut query = Document::new();

        if let Some(project_id) = filter.project_id {
            query.insert("project_id", project_id.to_string());
        }
        if let Some(status) = filter.status {
            query.insert("status", status.to_string());
        }
        if let Some(priority) = filter.priority {
            query.insert("priority", priority.to_string());
        }

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .limit(filter.limit as i64)
            .skip(filter.offset as u64)
            .build();

        let cursor = self
            .collection
            .find(query)
            .with_options(options)
            .await
            .map_err(|e| TaskError::Internal(format!("MongoDB find error: {e}")))?;

        let docs: Vec<Document> = cursor
            .try_collect()
            .await
            .map_err(|e| TaskError::Internal(format!("MongoDB cursor error: {e}")))?;

        docs.into_iter().map(task_from_doc).collect()
    }

    async fn update(&self, id: Uuid, input: UpdateTask) -> TaskResult<Task> {
        let mut update_doc = Document::new();

        if let Some(title) = &input.title {
            update_doc.insert("title", title);
        }
        if let Some(description) = &input.description {
            update_doc.insert("description", description);
        }
        if let Some(priority) = input.priority {
            update_doc.insert("priority", priority.to_string());
        }
        if let Some(status) = input.status {
            update_doc.insert("status", status.to_string());
        }
        if let Some(project_id) = &input.project_id {
            match project_id {
                Some(pid) => {
                    update_doc.insert("project_id", pid.to_string());
                }
                None => {
                    update_doc.insert("project_id", bson::Bson::Null);
                }
            }
        }
        if let Some(due_date) = &input.due_date {
            match due_date {
                Some(dt) => {
                    update_doc.insert(
                        "due_date",
                        bson::DateTime::from_millis(dt.timestamp_millis()),
                    );
                }
                None => {
                    update_doc.insert("due_date", bson::Bson::Null);
                }
            }
        }

        update_doc.insert("updated_at", bson::DateTime::now());

        let result = self
            .collection
            .find_one_and_update(
                doc! { "_id": id.to_string() },
                doc! { "$set": update_doc },
            )
            .return_document(mongodb::options::ReturnDocument::After)
            .await
            .map_err(|e| TaskError::Internal(format!("MongoDB update error: {e}")))?;

        match result {
            Some(doc) => task_from_doc(doc),
            None => Err(TaskError::NotFound(id)),
        }
    }

    async fn delete(&self, id: Uuid) -> TaskResult<bool> {
        let result = self
            .collection
            .delete_one(doc! { "_id": id.to_string() })
            .await
            .map_err(|e| TaskError::Internal(format!("MongoDB delete error: {e}")))?;

        Ok(result.deleted_count > 0)
    }

    async fn count(&self) -> TaskResult<usize> {
        let count = self
            .collection
            .count_documents(doc! {})
            .await
            .map_err(|e| TaskError::Internal(format!("MongoDB count error: {e}")))?;

        Ok(count as usize)
    }

    async fn count_by_project(&self, project_id: Uuid) -> TaskResult<usize> {
        let count = self
            .collection
            .count_documents(doc! { "project_id": project_id.to_string() })
            .await
            .map_err(|e| TaskError::Internal(format!("MongoDB count error: {e}")))?;

        Ok(count as usize)
    }
}
