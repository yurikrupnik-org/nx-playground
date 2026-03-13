//! Tasks gRPC service implementation
//!
//! This module contains the TasksServiceImpl struct and its gRPC trait implementation.
//! Handlers are kept minimal by leveraging From/TryFrom trait conversions defined in
//! domain_tasks::conversions.

use std::pin::Pin;
use std::sync::Arc;

use domain_tasks::{
    CreateTask, TaskFilter, TaskRepository, TaskService, UpdateTask, conversions as conv,
};
use grpc_client::ToTonicResult;
use rpc::tasks::{
    CreateRequest, CreateResponse, DeleteByIdRequest, DeleteByIdResponse, GetByIdRequest,
    GetByIdResponse, ListRequest, ListResponse, ListStreamRequest, ListStreamResponse,
    UpdateByIdRequest, UpdateByIdResponse, tasks_service_server::TasksService,
};
use tokio_stream::Stream;
use tonic::{Request, Response, Status};
use tracing::info;

/// Type alias for streaming responses
type TaskStream = Pin<Box<dyn Stream<Item = Result<ListStreamResponse, Status>> + Send>>;

/// gRPC service implementation for tasks
///
/// Wraps the domain TaskService and handles proto ↔ domain conversions.
/// Generic over the repository type for testability.
pub struct TasksServiceImpl<R>
where
    R: TaskRepository + 'static,
{
    service: Arc<TaskService<R>>,
}

impl<R> TasksServiceImpl<R>
where
    R: TaskRepository + 'static,
{
    /// Create a new tasks service implementation
    pub fn new(service: TaskService<R>) -> Self {
        Self {
            service: Arc::new(service),
        }
    }
}

#[tonic::async_trait]
impl<R> TasksService for TasksServiceImpl<R>
where
    R: TaskRepository + 'static,
{
    async fn create(
        &self,
        request: Request<CreateRequest>,
    ) -> Result<Response<CreateResponse>, Status> {
        let input: CreateTask = request.into_inner().try_into().to_tonic()?;
        let task = self
            .service
            .create_task(input)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(task.into()))
    }

    async fn get_by_id(
        &self,
        request: Request<GetByIdRequest>,
    ) -> Result<Response<GetByIdResponse>, Status> {
        let id = conv::bytes_to_uuid(&request.into_inner().id).to_tonic()?;
        let task = self
            .service
            .get_task(id)
            .await
            .map_err(|e| Status::not_found(e.to_string()))?;
        Ok(Response::new(task.into()))
    }

    async fn delete_by_id(
        &self,
        request: Request<DeleteByIdRequest>,
    ) -> Result<Response<DeleteByIdResponse>, Status> {
        let id = conv::bytes_to_uuid(&request.into_inner().id).to_tonic()?;
        self.service
            .delete_task(id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        info!("Deleted task: {}", id);
        Ok(Response::new(DeleteByIdResponse {}))
    }

    async fn update_by_id(
        &self,
        request: Request<UpdateByIdRequest>,
    ) -> Result<Response<UpdateByIdResponse>, Status> {
        let mut req = request.into_inner();
        let id = conv::bytes_to_uuid(&req.id).to_tonic()?;
        req.id = vec![]; // Clear ID before conversion
        let input: UpdateTask = req.try_into().to_tonic()?;
        let task = self
            .service
            .update_task(id, input)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(task.into()))
    }

    async fn list(&self, request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let req = request.into_inner();
        let filter = TaskFilter {
            project_id: conv::opt_bytes_to_uuid(req.project_id).to_tonic()?,
            status: req.status.map(|s| s.try_into()).transpose().to_tonic()?,
            priority: req.priority.map(|p| p.try_into()).transpose().to_tonic()?,
            limit: req.limit as usize,
            offset: req.offset as usize,
        };
        let tasks = self
            .service
            .list_tasks(filter)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let data: Vec<CreateResponse> = tasks.into_iter().map(|task| task.into()).collect();
        Ok(Response::new(ListResponse { data }))
    }

    type ListStreamStream = TaskStream;

    async fn list_stream(
        &self,
        request: Request<ListStreamRequest>,
    ) -> Result<Response<Self::ListStreamStream>, Status> {
        let req = request.into_inner();
        let filter = TaskFilter {
            project_id: conv::opt_bytes_to_uuid(req.project_id).to_tonic()?,
            status: req.status.map(|s| s.try_into()).transpose().to_tonic()?,
            priority: req.priority.map(|p| p.try_into()).transpose().to_tonic()?,
            limit: req.limit as usize,
            offset: 0,
        };
        let tasks = self
            .service
            .list_tasks(filter)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let stream = tokio_stream::iter(tasks.into_iter().map(|task| Ok(task.into())));
        Ok(Response::new(Box::pin(stream)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use domain_tasks::{Task, TaskError, TaskPriority, TaskStatus};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    /// Mock repository for testing
    #[derive(Clone)]
    struct MockTaskRepository {
        tasks: Arc<Mutex<HashMap<Uuid, Task>>>,
    }

    impl MockTaskRepository {
        fn new() -> Self {
            Self {
                tasks: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        fn with_task(task: Task) -> Self {
            let mut tasks = HashMap::new();
            tasks.insert(task.id, task);
            Self {
                tasks: Arc::new(Mutex::new(tasks)),
            }
        }
    }

    #[tonic::async_trait]
    impl TaskRepository for MockTaskRepository {
        async fn create(&self, input: CreateTask) -> Result<Task, TaskError> {
            let task = Task {
                id: Uuid::new_v4(),
                title: input.title,
                description: input.description,
                project_id: input.project_id,
                priority: input.priority,
                status: input.status,
                due_date: input.due_date,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            self.tasks.lock().unwrap().insert(task.id, task.clone());
            Ok(task)
        }

        async fn get_by_id(&self, id: Uuid) -> Result<Option<Task>, TaskError> {
            Ok(self.tasks.lock().unwrap().get(&id).cloned())
        }

        async fn update(&self, id: Uuid, input: UpdateTask) -> Result<Task, TaskError> {
            let mut tasks = self.tasks.lock().unwrap();
            let task = tasks.get_mut(&id).ok_or(TaskError::NotFound(id))?;

            if let Some(title) = input.title {
                task.title = title;
            }
            if let Some(description) = input.description {
                task.description = description;
            }
            if let Some(project_id) = input.project_id {
                task.project_id = project_id;
            }
            if let Some(priority) = input.priority {
                task.priority = priority;
            }
            if let Some(status) = input.status {
                task.status = status;
            }
            if let Some(due_date) = input.due_date {
                task.due_date = due_date;
            }
            task.updated_at = Utc::now();

            Ok(task.clone())
        }

        async fn delete(&self, id: Uuid) -> Result<bool, TaskError> {
            Ok(self.tasks.lock().unwrap().remove(&id).is_some())
        }

        async fn list(&self, filter: TaskFilter) -> Result<Vec<Task>, TaskError> {
            let tasks = self.tasks.lock().unwrap();
            let mut result: Vec<Task> = tasks
                .values()
                .filter(|task| {
                    if let Some(project_id) = filter.project_id
                        && task.project_id != Some(project_id)
                    {
                        return false;
                    }
                    if let Some(status) = filter.status
                        && task.status != status
                    {
                        return false;
                    }
                    if let Some(priority) = filter.priority
                        && task.priority != priority
                    {
                        return false;
                    }
                    true
                })
                .cloned()
                .collect();

            result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Ok(result
                .into_iter()
                .skip(filter.offset)
                .take(filter.limit)
                .collect())
        }

        async fn count(&self) -> Result<usize, TaskError> {
            Ok(self.tasks.lock().unwrap().len())
        }

        async fn count_by_project(&self, project_id: Uuid) -> Result<usize, TaskError> {
            Ok(self
                .tasks
                .lock()
                .unwrap()
                .values()
                .filter(|task| task.project_id == Some(project_id))
                .count())
        }
    }

    fn create_test_service() -> TasksServiceImpl<MockTaskRepository> {
        let repository = MockTaskRepository::new();
        let service = TaskService::new(repository);
        TasksServiceImpl::new(service)
    }

    fn create_test_task() -> Task {
        Task {
            id: Uuid::new_v4(),
            title: "Test Task".to_string(),
            description: "Test Description".to_string(),
            project_id: None,
            priority: TaskPriority::Medium,
            status: TaskStatus::Todo,
            due_date: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_create_task_success() {
        let service = create_test_service();

        let request = Request::new(CreateRequest {
            title: "New Task".to_string(),
            description: "Task Description".to_string(),
            project_id: None,
            priority: 2, // Medium (proto: 2 = MEDIUM)
            status: 1,   // Todo (proto: 1 = TODO)
            due_date: None,
        });

        let response = service.create(request).await;
        if let Err(ref e) = response {
            eprintln!("Create failed with error: {:?}", e);
        }
        assert!(response.is_ok(), "Create task should succeed");

        let task = response.unwrap().into_inner();
        assert_eq!(task.title, "New Task");
        assert_eq!(task.description, "Task Description");
    }

    #[tokio::test]
    async fn test_create_task_with_invalid_priority() {
        let service = create_test_service();

        let request = Request::new(CreateRequest {
            title: "New Task".to_string(),
            description: "Task Description".to_string(),
            project_id: None,
            priority: 999, // Invalid priority
            status: 1,     // Valid status
            due_date: None,
        });

        let response = service.create(request).await;
        assert!(response.is_err());
        assert_eq!(response.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_get_task_success() {
        let task = create_test_task();
        let task_id = task.id;

        let repository = MockTaskRepository::with_task(task);
        let domain_service = TaskService::new(repository);
        let service = TasksServiceImpl::new(domain_service);

        let request = Request::new(GetByIdRequest {
            id: conv::uuid_to_bytes(task_id),
        });

        let response = service.get_by_id(request).await;
        assert!(response.is_ok());

        let result = response.unwrap().into_inner();
        assert_eq!(result.title, "Test Task");
    }

    #[tokio::test]
    async fn test_get_task_not_found() {
        let service = create_test_service();

        let request = Request::new(GetByIdRequest {
            id: conv::uuid_to_bytes(Uuid::new_v4()),
        });

        let response = service.get_by_id(request).await;
        assert!(response.is_err());
        assert_eq!(response.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_get_task_invalid_uuid() {
        let service = create_test_service();

        let request = Request::new(GetByIdRequest {
            id: vec![1, 2, 3], // Invalid UUID bytes
        });

        let response = service.get_by_id(request).await;
        assert!(response.is_err());
        assert_eq!(response.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_update_task_success() {
        let task = create_test_task();
        let task_id = task.id;

        let repository = MockTaskRepository::with_task(task);
        let domain_service = TaskService::new(repository);
        let service = TasksServiceImpl::new(domain_service);

        let request = Request::new(UpdateByIdRequest {
            id: conv::uuid_to_bytes(task_id),
            title: Some("Updated Title".to_string()),
            description: None,
            completed: None,
            project_id: None,
            priority: Some(2), // High
            status: Some(1),   // InProgress
            due_date: None,
        });

        let response = service.update_by_id(request).await;
        assert!(response.is_ok());

        let result = response.unwrap().into_inner();
        assert_eq!(result.title, "Updated Title");
    }

    #[tokio::test]
    async fn test_delete_task_success() {
        let task = create_test_task();
        let task_id = task.id;

        let repository = MockTaskRepository::with_task(task);
        let domain_service = TaskService::new(repository);
        let service = TasksServiceImpl::new(domain_service);

        let request = Request::new(DeleteByIdRequest {
            id: conv::uuid_to_bytes(task_id),
        });

        let response = service.delete_by_id(request).await;
        assert!(response.is_ok());
    }

    #[tokio::test]
    async fn test_delete_task_not_found() {
        let service = create_test_service();

        let request = Request::new(DeleteByIdRequest {
            id: conv::uuid_to_bytes(Uuid::new_v4()),
        });

        let response = service.delete_by_id(request).await;
        assert!(response.is_err());
        assert_eq!(response.unwrap_err().code(), tonic::Code::Internal);
    }

    #[tokio::test]
    async fn test_list_tasks_empty() {
        let service = create_test_service();

        let request = Request::new(ListRequest {
            project_id: None,
            status: None,
            priority: None,
            completed: None,
            limit: 10,
            offset: 0,
        });

        let response = service.list(request).await;
        assert!(response.is_ok());

        let result = response.unwrap().into_inner();
        assert_eq!(result.data.len(), 0);
    }

    #[tokio::test]
    async fn test_list_tasks_with_filters() {
        let task1 = Task {
            priority: TaskPriority::High,
            status: TaskStatus::InProgress,
            ..create_test_task()
        };
        let task2 = Task {
            priority: TaskPriority::Low,
            status: TaskStatus::Done,
            ..create_test_task()
        };

        let mut tasks = HashMap::new();
        tasks.insert(task1.id, task1);
        tasks.insert(task2.id, task2);

        let repository = MockTaskRepository {
            tasks: Arc::new(Mutex::new(tasks)),
        };
        let domain_service = TaskService::new(repository);
        let service = TasksServiceImpl::new(domain_service);

        let request = Request::new(ListRequest {
            project_id: None,
            status: Some(2),   // InProgress (proto: 2 = IN_PROGRESS)
            priority: Some(3), // High (proto: 3 = HIGH)
            completed: None,
            limit: 10,
            offset: 0,
        });

        let response = service.list(request).await;
        assert!(response.is_ok());

        let result = response.unwrap().into_inner();
        assert_eq!(result.data.len(), 1, "Should find 1 task matching filters");
        assert_eq!(result.data[0].priority, 3); // High
    }

    #[tokio::test]
    async fn test_list_stream_success() {
        use tokio_stream::StreamExt;

        let task = create_test_task();
        let repository = MockTaskRepository::with_task(task);
        let domain_service = TaskService::new(repository);
        let service = TasksServiceImpl::new(domain_service);

        let request = Request::new(ListStreamRequest {
            project_id: None,
            status: None,
            priority: None,
            completed: None,
            limit: 10,
        });

        let response = service.list_stream(request).await;
        assert!(response.is_ok());

        let mut stream = response.unwrap().into_inner();
        let mut count = 0;
        while let Some(result) = stream.next().await {
            assert!(result.is_ok());
            count += 1;
        }
        assert_eq!(count, 1);
    }
}
