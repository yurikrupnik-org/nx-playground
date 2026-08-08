//! Tasks gRPC service implementation
//!
//! This module contains the TasksServiceImpl struct and its gRPC trait implementation.
//! Handlers are kept minimal by leveraging From/TryFrom trait conversions defined in
//! domain_tasks::conversions.

use std::pin::Pin;
use std::sync::Arc;

use domain_tasks::{
    CreateTask, TaskError, TaskFilter, TaskRepository, TaskService, UpdateTask, conversions as conv,
};
use grpc_client::ToTonicResult;
use rpc::tasks::v1::{
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
    auth: crate::auth::CallerAuth,
}

impl<R> TasksServiceImpl<R>
where
    R: TaskRepository + 'static,
{
    /// Create a new tasks service implementation
    pub fn new(service: TaskService<R>, auth: crate::auth::CallerAuth) -> Self {
        Self {
            service: Arc::new(service),
            auth,
        }
    }
}

/// Map a domain error onto a wire status.
///
/// The code is load-bearing: the BFF turns it straight into an HTTP status, so a
/// blanket `Status::internal` (or a blanket `not_found`) misreports every failure.
/// Database details are logged, not returned.
fn to_status(err: TaskError) -> Status {
    match err {
        TaskError::NotFound(id) => Status::not_found(format!("task {id} not found")),
        TaskError::Validation(msg) => Status::invalid_argument(msg),
        TaskError::Internal(msg) => Status::internal(msg),
        TaskError::Database(e) => {
            tracing::error!(error = %e, "database error");
            Status::internal("database error")
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
        let scope = self.auth.scope(&request).await?;
        let input: CreateTask = request.into_inner().try_into()?;
        let task = self
            .service
            .create_task(scope, input)
            .await
            .map_err(to_status)?;
        Ok(Response::new(task.into()))
    }

    async fn get_by_id(
        &self,
        request: Request<GetByIdRequest>,
    ) -> Result<Response<GetByIdResponse>, Status> {
        let scope = self.auth.scope(&request).await?;
        let id = conv::bytes_to_uuid(&request.into_inner().id).to_tonic()?;
        let task = self
            .service
            .get_task(&scope.org_ref, id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(task.into()))
    }

    async fn delete_by_id(
        &self,
        request: Request<DeleteByIdRequest>,
    ) -> Result<Response<DeleteByIdResponse>, Status> {
        let scope = self.auth.scope(&request).await?;
        let id = conv::bytes_to_uuid(&request.into_inner().id).to_tonic()?;
        self.service
            .delete_task(&scope.org_ref, id)
            .await
            .map_err(to_status)?;
        info!("Deleted task: {}", id);
        Ok(Response::new(DeleteByIdResponse {}))
    }

    async fn update_by_id(
        &self,
        request: Request<UpdateByIdRequest>,
    ) -> Result<Response<UpdateByIdResponse>, Status> {
        let scope = self.auth.scope(&request).await?;
        let mut req = request.into_inner();
        let id = conv::bytes_to_uuid(&req.id).to_tonic()?;
        req.id = vec![]; // Clear ID before conversion
        let input: UpdateTask = req.try_into()?;
        let task = self
            .service
            .update_task(&scope.org_ref, id, input)
            .await
            .map_err(to_status)?;
        Ok(Response::new(task.into()))
    }

    async fn list(&self, request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let scope = self.auth.scope(&request).await?;
        let req = request.into_inner();
        let filter = TaskFilter {
            mine: req.mine,
            project_id: conv::opt_bytes_to_uuid(req.project_id).to_tonic()?,
            status: req.status.map(|s| s.try_into()).transpose()?,
            priority: req.priority.map(|p| p.try_into()).transpose()?,
            completed: req.completed,
            limit: req.limit as usize,
            offset: req.offset as usize,
        };
        let tasks = self
            .service
            .list_tasks(&scope, filter)
            .await
            .map_err(to_status)?;
        let data: Vec<CreateResponse> = tasks.into_iter().map(|task| task.into()).collect();
        Ok(Response::new(ListResponse { data }))
    }

    type ListStreamStream = TaskStream;

    async fn list_stream(
        &self,
        request: Request<ListStreamRequest>,
    ) -> Result<Response<Self::ListStreamStream>, Status> {
        let scope = self.auth.scope(&request).await?;
        let req = request.into_inner();
        let filter = TaskFilter {
            mine: req.mine,
            project_id: conv::opt_bytes_to_uuid(req.project_id).to_tonic()?,
            status: req.status.map(|s| s.try_into()).transpose()?,
            priority: req.priority.map(|p| p.try_into()).transpose()?,
            completed: req.completed,
            limit: req.limit as usize,
            offset: 0,
        };
        let tasks = self
            .service
            .list_tasks(&scope, filter)
            .await
            .map_err(to_status)?;
        let stream = tokio_stream::iter(tasks.into_iter().map(|task| Ok(task.into())));
        Ok(Response::new(Box::pin(stream)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::CallerAuth;
    use chrono::Utc;
    use domain_tasks::TaskScope;
    use domain_tasks::{Task, TaskError, TaskPriority, TaskStatus};
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn test_org() -> String {
        "org_01TESTORG".to_string()
    }

    fn test_user() -> String {
        "user_01TESTUSER".to_string()
    }

    /// The scope a verified caller token would yield.
    fn test_scope() -> TaskScope {
        TaskScope {
            org_ref: test_org(),
            user_ref: test_user(),
        }
    }

    /// Mock repository for testing; honors the org scope like the real one.
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
        async fn create(&self, scope: TaskScope, input: CreateTask) -> Result<Task, TaskError> {
            let task = Task {
                id: Uuid::new_v4(),
                org_ref: scope.org_ref.clone(),
                user_ref: scope.user_ref.clone(),
                title: input.title,
                description: input.description,
                completed: false,
                project_id: input.project_id,
                priority: input.priority,
                status: input.status,
                due_date: input.due_date,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            self.tasks.lock().insert(task.id, task.clone());
            Ok(task)
        }

        async fn get_by_id(&self, org_ref: &str, id: Uuid) -> Result<Option<Task>, TaskError> {
            Ok(self
                .tasks
                .lock()
                .get(&id)
                .filter(|t| t.org_ref == org_ref)
                .cloned())
        }

        async fn update(
            &self,
            org_ref: &str,
            id: Uuid,
            input: UpdateTask,
        ) -> Result<Task, TaskError> {
            let mut tasks = self.tasks.lock();
            let task = tasks
                .get_mut(&id)
                .filter(|t| t.org_ref == org_ref)
                .ok_or(TaskError::NotFound(id))?;

            if let Some(title) = input.title {
                task.title = title;
            }
            if let Some(description) = input.description {
                task.description = description;
            }
            if let Some(completed) = input.completed {
                task.completed = completed;
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

        async fn delete(&self, org_ref: &str, id: Uuid) -> Result<bool, TaskError> {
            let mut tasks = self.tasks.lock();
            if tasks.get(&id).is_some_and(|t| t.org_ref == org_ref) {
                tasks.remove(&id);
                Ok(true)
            } else {
                Ok(false)
            }
        }

        async fn list(
            &self,
            scope: &TaskScope,
            filter: TaskFilter,
        ) -> Result<Vec<Task>, TaskError> {
            let tasks = self.tasks.lock();
            let mut result: Vec<Task> = tasks
                .values()
                .filter(|task| {
                    if task.org_ref != scope.org_ref {
                        return false;
                    }
                    if filter.mine && task.user_ref != scope.user_ref {
                        return false;
                    }
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
                    if let Some(completed) = filter.completed
                        && task.completed != completed
                    {
                        return false;
                    }
                    true
                })
                .cloned()
                .collect();

            result.sort_by_key(|b| std::cmp::Reverse(b.created_at));
            Ok(result
                .into_iter()
                .skip(filter.offset)
                .take(filter.limit)
                .collect())
        }

        async fn count(&self, org_ref: &str) -> Result<usize, TaskError> {
            Ok(self
                .tasks
                .lock()
                .values()
                .filter(|t| t.org_ref == org_ref)
                .count())
        }

        async fn count_by_project(
            &self,
            org_ref: &str,
            project_id: Uuid,
        ) -> Result<usize, TaskError> {
            Ok(self
                .tasks
                .lock()
                .values()
                .filter(|task| task.org_ref == org_ref && task.project_id == Some(project_id))
                .count())
        }
    }

    fn create_test_service() -> TasksServiceImpl<MockTaskRepository> {
        let repository = MockTaskRepository::new();
        let service = TaskService::new(repository);
        TasksServiceImpl::new(service, CallerAuth::fixed(test_scope()))
    }

    fn create_test_task() -> Task {
        Task {
            id: Uuid::new_v4(),
            org_ref: test_org(),
            user_ref: test_user(),
            title: "Test Task".to_string(),
            description: "Test Description".to_string(),
            completed: false,
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
        assert!(!task.completed);
        assert_eq!(task.org_ref, test_org());
        assert_eq!(task.user_ref, test_user());
    }

    #[tokio::test]
    async fn test_create_without_token_is_unauthenticated() {
        // A service wired to the real verifier, called with no `authorization`
        // metadata. Identity is not a request field any more, so the only way to act
        // on a tenant is to present a token - this is the boundary property.
        let service = TasksServiceImpl::new(
            TaskService::new(MockTaskRepository::new()),
            CallerAuth::new(Arc::new(oidc_auth::OidcVerifier::new(
                oidc_auth::VerifierConfig::workos("client_test", "https://example.invalid"),
            ))),
        );

        let request = Request::new(CreateRequest {
            title: "New Task".to_string(),
            description: String::new(),
            project_id: None,
            priority: 2,
            status: 1,
            due_date: None,
        });

        let response = service.create(request).await;
        assert!(response.is_err());
        assert_eq!(
            response.unwrap_err().code(),
            tonic::Code::Unauthenticated,
            "an unauthenticated caller must not reach the repository"
        );
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
        let service = TasksServiceImpl::new(domain_service, CallerAuth::fixed(test_scope()));

        let request = Request::new(GetByIdRequest {
            id: conv::uuid_to_bytes(task_id),
        });

        let response = service.get_by_id(request).await;
        assert!(response.is_ok());

        let result = response.unwrap().into_inner();
        assert_eq!(result.title, "Test Task");
    }

    #[tokio::test]
    async fn test_get_task_cross_org_not_found() {
        let task = create_test_task();
        let task_id = task.id;

        let repository = MockTaskRepository::with_task(task);
        let domain_service = TaskService::new(repository);
        // A caller whose *verified token* resolves to a different organization. The
        // request is byte-identical to the owner's - only the token differs, which is
        // the whole point of deriving scope from it.
        let service = TasksServiceImpl::new(
            domain_service,
            CallerAuth::fixed(TaskScope {
                org_ref: "org_01OTHER".to_string(),
                user_ref: "user_01OTHER".to_string(),
            }),
        );

        let request = Request::new(GetByIdRequest {
            id: conv::uuid_to_bytes(task_id),
        });

        let response = service.get_by_id(request).await;
        assert!(response.is_err(), "another org's task must not be readable");
        assert_eq!(response.unwrap_err().code(), tonic::Code::NotFound);
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
        let service = TasksServiceImpl::new(domain_service, CallerAuth::fixed(test_scope()));

        let request = Request::new(UpdateByIdRequest {
            id: conv::uuid_to_bytes(task_id),
            title: Some("Updated Title".to_string()),
            description: None,
            completed: Some(true),
            project_id: None,
            priority: Some(2), // High
            status: Some(1),   // InProgress
            due_date: None,
        });

        let response = service.update_by_id(request).await;
        assert!(response.is_ok());

        let result = response.unwrap().into_inner();
        assert_eq!(result.title, "Updated Title");
        assert!(result.completed);
    }

    #[tokio::test]
    async fn test_delete_task_success() {
        let task = create_test_task();
        let task_id = task.id;

        let repository = MockTaskRepository::with_task(task);
        let domain_service = TaskService::new(repository);
        let service = TasksServiceImpl::new(domain_service, CallerAuth::fixed(test_scope()));

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
        // A missing task is the caller's mistake, not ours: the BFF turns this code
        // straight into an HTTP status, so it must be NotFound (404), never Internal.
        assert_eq!(response.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_list_tasks_empty() {
        let service = create_test_service();

        let request = Request::new(ListRequest {
            mine: false,
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
    async fn test_list_without_token_is_unauthenticated() {
        // `list` has no org field to omit any more, so the failure mode that matters
        // is an absent credential. Wired to the real verifier, no metadata attached.
        let service = TasksServiceImpl::new(
            TaskService::new(MockTaskRepository::new()),
            CallerAuth::new(Arc::new(oidc_auth::OidcVerifier::new(
                oidc_auth::VerifierConfig::workos("client_test", "https://example.invalid"),
            ))),
        );

        let request = Request::new(ListRequest {
            mine: false,
            project_id: None,
            status: None,
            priority: None,
            completed: None,
            limit: 10,
            offset: 0,
        });

        let response = service.list(request).await;
        assert!(response.is_err());
        assert_eq!(response.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn test_list_tasks_scoped_to_org() {
        let mine = create_test_task();
        let foreign = Task {
            org_ref: "org_01OTHER".to_string(),
            ..create_test_task()
        };

        let mut tasks = HashMap::new();
        tasks.insert(mine.id, mine.clone());
        tasks.insert(foreign.id, foreign);

        let repository = MockTaskRepository {
            tasks: Arc::new(Mutex::new(tasks)),
        };
        let domain_service = TaskService::new(repository);
        let service = TasksServiceImpl::new(domain_service, CallerAuth::fixed(test_scope()));

        let request = Request::new(ListRequest {
            mine: false,
            project_id: None,
            status: None,
            priority: None,
            completed: None,
            limit: 10,
            offset: 0,
        });

        let response = service.list(request).await.unwrap().into_inner();
        assert_eq!(response.data.len(), 1, "Foreign-org task must not leak");
        assert_eq!(response.data[0].id, conv::uuid_to_bytes(mine.id));
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
        let service = TasksServiceImpl::new(domain_service, CallerAuth::fixed(test_scope()));

        let request = Request::new(ListRequest {
            mine: false,
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
        let service = TasksServiceImpl::new(domain_service, CallerAuth::fixed(test_scope()));

        let request = Request::new(ListStreamRequest {
            mine: false,
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
