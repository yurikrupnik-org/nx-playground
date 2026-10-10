use std::sync::Arc;

use axum::Router;
use domain_projects::{PgProjectRepository, ProjectService, handlers};

pub fn router(state: &crate::state::AppState) -> Router {
    let repository = PgProjectRepository::new(state.db.clone());
    let service = ProjectService::new(repository, Arc::clone(&state.project_events));
    handlers::router(service)
}
