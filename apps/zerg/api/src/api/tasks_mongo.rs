use axum::Router;
use domain_tasks::{MongoTaskRepository, TaskService, handlers};

pub fn router(mongo_db: mongodb::Database) -> Router {
    let repository = MongoTaskRepository::new(mongo_db);
    let service = TaskService::new(repository);
    handlers::direct_router(service)
}
