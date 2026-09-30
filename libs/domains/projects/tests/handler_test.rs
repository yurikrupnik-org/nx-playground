//! Handler tests for Projects domain
//!
//! These tests verify that HTTP handlers work correctly:
//! - Request deserialization (JSON → Rust structs)
//! - Response serialization (Rust structs → JSON)
//! - HTTP status codes
//! - Error responses
//!
//! Unlike E2E tests, these test ONLY the projects domain handlers,
//! not the full application with routing, auth middleware, etc.

#![allow(clippy::unwrap_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::Arc;

use domain_projects::*;
use http_body_util::BodyExt;
use serde_json::json;
use test_utils::{TestDataBuilder, TestDatabase};
use tower::ServiceExt; // For oneshot()

// Helper to parse JSON response body
async fn json_body<T: serde::de::DeserializeOwned>(body: Body) -> T {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn test_create_project_handler_returns_201() {
    let db = TestDatabase::new().await;
    let builder = TestDataBuilder::from_test_name("handler_create_201");
    db.create_test_user(builder.user_id()).await;

    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));
    let app = handlers::router(service);

    let request = Request::builder()
        .method("POST")
        .uri("/")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({
                "name": builder.name("project", "test"),
                "user_id": builder.user_id(),
                "description": "Handler test",
                "cloud_provider": "aws",
                "region": "us-east-1",
                "environment": "development",
                "budget_limit": 100.0,
                "tags": []
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let project: Project = json_body(response.into_body()).await;
    assert_eq!(project.name, builder.name("project", "test"));
    assert_eq!(project.cloud_provider, CloudProvider::Aws);
}

#[tokio::test]
async fn test_create_project_handler_validates_input() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));
    let app = handlers::router(service);

    let builder = TestDataBuilder::from_test_name("handler_validate");

    // Invalid name (empty string)
    let request = Request::builder()
        .method("POST")
        .uri("/")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({
                "name": "",  // Invalid!
                "user_id": builder.user_id(),
                "description": "",
                "cloud_provider": "aws",
                "region": "us-east-1"
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_project_handler_enforces_free_tier_limit() {
    let db = TestDatabase::new().await;
    let builder = TestDataBuilder::from_test_name("handler_free_tier");
    let user_id = builder.user_id();
    db.create_test_user(user_id).await;

    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));

    // Create 3 projects directly via service
    for i in 0..3 {
        let input = CreateProject {
            name: builder.name("project", &format!("p{i}")),
            user_id,
            description: String::new(),
            cloud_provider: CloudProvider::Aws,
            region: "us-east-1".to_string(),
            environment: Environment::Development,
            budget_limit: None,
            tags: vec![],
        };
        service.create_project(input).await.unwrap();
    }

    // Try to create 4th via HTTP handler
    let app = handlers::router(service);

    let request = Request::builder()
        .method("POST")
        .uri("/")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({
                "name": builder.name("project", "p4"),
                "user_id": user_id,
                "description": "",
                "cloud_provider": "aws",
                "region": "us-east-1"
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Free tier limit"));
}

#[tokio::test]
async fn test_get_project_handler_returns_200() {
    let db = TestDatabase::new().await;
    let builder = TestDataBuilder::from_test_name("handler_get_200");
    db.create_test_user(builder.user_id()).await;

    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));

    // Create a project
    let input = CreateProject {
        name: builder.name("project", "get-test"),
        user_id: builder.user_id(),
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };
    let created = service.create_project(input).await.unwrap();

    // Get it via handler
    let app = handlers::router(service);

    let request = Request::builder()
        .method("GET")
        .uri(format!("/{}", created.id))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let project: Project = json_body(response.into_body()).await;
    assert_eq!(project.id, created.id);
    assert_eq!(project.name, builder.name("project", "get-test"));
}

#[tokio::test]
async fn test_get_project_handler_returns_404_for_missing() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));
    let app = handlers::router(service);

    let missing_id = uuid::Uuid::now_v7();

    let request = Request::builder()
        .method("GET")
        .uri(format!("/{missing_id}"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_list_projects_handler_with_filters() {
    let db = TestDatabase::new().await;
    let builder = TestDataBuilder::from_test_name("handler_list_filters");
    let user_id = builder.user_id();
    db.create_test_user(user_id).await;

    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));

    // Create 2 AWS and 1 GCP project
    for i in 0..2 {
        let input = CreateProject {
            name: builder.name("project", &format!("aws-{i}")),
            user_id,
            description: String::new(),
            cloud_provider: CloudProvider::Aws,
            region: "us-east-1".to_string(),
            environment: Environment::Development,
            budget_limit: None,
            tags: vec![],
        };
        service.create_project(input).await.unwrap();
    }

    let input = CreateProject {
        name: builder.name("project", "gcp-0"),
        user_id,
        description: String::new(),
        cloud_provider: CloudProvider::Gcp,
        region: "us-central1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };
    service.create_project(input).await.unwrap();

    let app = handlers::router(service);

    // Filter by cloud_provider=aws
    let request = Request::builder()
        .method("GET")
        .uri(format!("/?user_id={user_id}&cloud_provider=aws"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let projects: Vec<Project> = json_body(response.into_body()).await;
    assert_eq!(projects.len(), 2);
    assert!(
        projects
            .iter()
            .all(|p| p.cloud_provider == CloudProvider::Aws)
    );
}

#[tokio::test]
async fn test_delete_project_handler_returns_204() {
    let db = TestDatabase::new().await;
    let builder = TestDataBuilder::from_test_name("handler_delete");
    db.create_test_user(builder.user_id()).await;

    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));

    // Create a project
    let input = CreateProject {
        name: builder.name("project", "delete-test"),
        user_id: builder.user_id(),
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };
    let created = service.create_project(input).await.unwrap();

    let app = handlers::router(service);

    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/{}", created.id))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}
