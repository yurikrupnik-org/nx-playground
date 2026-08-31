//! Integration tests for Projects domain
//!
//! These tests use real PostgreSQL via testcontainers to ensure:
//! - Database queries work correctly
//! - Constraints are enforced
//! - Transactions behave as expected
//! - Concurrent operations are handled properly

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use domain_projects::*;
use test_utils::{TestDataBuilder, TestDatabase, assertions::*};
use uuid::Uuid;

// ============================================================================
// Repository Tests
// ============================================================================

#[tokio::test]
async fn test_create_and_get_project() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let builder = TestDataBuilder::from_test_name("create_and_get");

    // Create test user first (required for foreign key constraint)
    let user_id = db.create_test_user(builder.user_id()).await;

    let input = CreateProject {
        name: builder.name("project", "main"),
        user_id,
        description: "Integration test project".to_string(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: Some(100.0),
        tags: vec![Tag {
            key: "environment".to_string(),
            value: "test".to_string(),
        }],
    };

    // Create project
    let created = repo.create(input.clone()).await.unwrap();

    assert_eq!(created.name, input.name);
    assert_uuid_eq(created.user_id, input.user_id, "user_id");
    assert_eq!(created.cloud_provider, CloudProvider::Aws);
    assert_eq!(created.environment, Environment::Development);
    assert_eq!(created.status, ProjectStatus::Provisioning);
    assert_eq!(created.tags.len(), 1);

    // Retrieve project
    let retrieved = repo.get_by_id(created.id).await.unwrap();
    let retrieved = assert_some(retrieved, "project should exist");

    assert_uuid_eq(retrieved.id, created.id, "retrieved project id");
    assert_eq!(retrieved.name, created.name);
}

#[tokio::test]
async fn test_duplicate_name_constraint() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let builder = TestDataBuilder::from_test_name("duplicate_name");

    let user_id = db.create_test_user(builder.user_id()).await;
    let name = builder.name("project", "duplicate");

    let input = CreateProject {
        name: name.clone(),
        user_id,
        description: String::new(),
        cloud_provider: CloudProvider::Gcp,
        region: "europe-west1".to_string(),
        environment: Environment::Production,
        budget_limit: None,
        tags: vec![],
    };

    // First creation should succeed
    repo.create(input.clone()).await.unwrap();

    // Second creation with same name should fail
    let result = repo.create(input).await;
    assert!(
        matches!(result, Err(ProjectError::DuplicateName(_))),
        "Expected DuplicateName error, got {result:?}"
    );
}

#[tokio::test]
async fn test_duplicate_name_different_users() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let builder = TestDataBuilder::from_test_name("duplicate_diff_users");

    let name = builder.name("project", "shared");
    let user1 = db.create_test_user(Uuid::now_v7()).await;
    let user2 = db.create_test_user(Uuid::now_v7()).await;

    let input1 = CreateProject {
        name: name.clone(),
        user_id: user1,
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };

    let input2 = CreateProject {
        name: name.clone(),
        user_id: user2,
        description: String::new(),
        cloud_provider: CloudProvider::Azure,
        region: "eastus".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };

    // Both should succeed (different users)
    let project1 = repo.create(input1).await.unwrap();
    let project2 = repo.create(input2).await.unwrap();

    assert_eq!(project1.name, project2.name);
    assert_ne!(project1.user_id, project2.user_id);
}

#[tokio::test]
async fn test_update_project() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let builder = TestDataBuilder::from_test_name("update");

    let user_id = db.create_test_user(builder.user_id()).await;

    let input = CreateProject {
        name: builder.name("project", "original"),
        user_id,
        description: "Original description".to_string(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: Some(100.0),
        tags: vec![],
    };

    let created = repo.create(input).await.unwrap();

    // Update multiple fields
    let update = UpdateProject {
        name: Some(builder.name("project", "updated")),
        description: Some("Updated description".to_string()),
        region: Some("us-west-2".to_string()),
        environment: Some(Environment::Production),
        status: Some(ProjectStatus::Active),
        budget_limit: Some(200.0),
        tags: Some(vec![Tag {
            key: "updated".to_string(),
            value: "true".to_string(),
        }]),
        enabled: Some(false),
    };

    let updated = repo.update(created.id, update).await.unwrap();

    assert_eq!(updated.name, builder.name("project", "updated"));
    assert_eq!(updated.description, "Updated description");
    assert_eq!(updated.region, "us-west-2");
    assert_eq!(updated.environment, Environment::Production);
    assert_eq!(updated.status, ProjectStatus::Active);
    assert_eq!(updated.budget_limit, Some(200.0));
    assert_eq!(updated.tags.len(), 1);
    assert!(!updated.enabled);
    assert!(updated.updated_at >= updated.created_at);
}

#[tokio::test]
async fn test_delete_project() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let builder = TestDataBuilder::from_test_name("delete");

    let user_id = db.create_test_user(builder.user_id()).await;

    let input = CreateProject {
        name: builder.name("project", "to-delete"),
        user_id,
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };

    let created = repo.create(input).await.unwrap();

    // Delete should succeed
    let deleted = repo.delete(created.id).await.unwrap();
    assert!(deleted, "delete should return true");

    // Project should no longer exist
    let retrieved = repo.get_by_id(created.id).await.unwrap();
    assert!(retrieved.is_none(), "project should be deleted");

    // Second delete should return false
    let deleted_again = repo.delete(created.id).await.unwrap();
    assert!(!deleted_again, "second delete should return false");
}

#[tokio::test]
async fn test_list_projects_with_filters() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let builder = TestDataBuilder::from_test_name("list_filters");

    let user_id = db.create_test_user(builder.user_id()).await;

    // Create multiple projects with different attributes
    let projects = vec![
        (
            CloudProvider::Aws,
            Environment::Development,
            ProjectStatus::Provisioning,
        ),
        (
            CloudProvider::Aws,
            Environment::Production,
            ProjectStatus::Active,
        ),
        (
            CloudProvider::Gcp,
            Environment::Development,
            ProjectStatus::Active,
        ),
        (
            CloudProvider::Azure,
            Environment::Staging,
            ProjectStatus::Suspended,
        ),
    ];

    for (i, (provider, env, status)) in projects.into_iter().enumerate() {
        let input = CreateProject {
            name: builder.name("project", &format!("project-{i}")),
            user_id,
            description: String::new(),
            cloud_provider: provider,
            region: "region".to_string(),
            environment: env,
            budget_limit: None,
            tags: vec![],
        };

        let created = repo.create(input).await.unwrap();

        // Update status if needed
        if status != ProjectStatus::Provisioning {
            repo.update(
                created.id,
                UpdateProject {
                    status: Some(status),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
    }

    // Test: Filter by user
    let filter = ProjectFilter {
        user_id: Some(user_id),
        ..Default::default()
    };
    let results = repo.list(filter).await.unwrap();
    assert_eq!(results.len(), 4, "should return all user's projects");

    // Test: Filter by cloud provider
    let filter = ProjectFilter {
        user_id: Some(user_id),
        cloud_provider: Some(CloudProvider::Aws),
        ..Default::default()
    };
    let results = repo.list(filter).await.unwrap();
    assert_eq!(results.len(), 2, "should return AWS projects");

    // Test: Filter by environment
    let filter = ProjectFilter {
        user_id: Some(user_id),
        environment: Some(Environment::Development),
        ..Default::default()
    };
    let results = repo.list(filter).await.unwrap();
    assert_eq!(results.len(), 2, "should return dev projects");

    // Test: Filter by status
    let filter = ProjectFilter {
        user_id: Some(user_id),
        status: Some(ProjectStatus::Active),
        ..Default::default()
    };
    let results = repo.list(filter).await.unwrap();
    assert_eq!(results.len(), 2, "should return active projects");

    // Test: Pagination
    let filter = ProjectFilter {
        user_id: Some(user_id),
        limit: 2,
        offset: 0,
        ..Default::default()
    };
    let page1 = repo.list(filter.clone()).await.unwrap();
    assert_eq!(page1.len(), 2, "first page should have 2 items");

    let filter = ProjectFilter {
        offset: 2,
        ..filter
    };
    let page2 = repo.list(filter).await.unwrap();
    assert_eq!(page2.len(), 2, "second page should have 2 items");
}

// ============================================================================
// Service Tests
// ============================================================================

#[tokio::test]
async fn test_service_validation() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));
    let builder = TestDataBuilder::from_test_name("service_validation");

    let user_id = db.create_test_user(builder.user_id()).await;

    // Test: Empty name should fail
    let input = CreateProject {
        name: "".to_string(),
        user_id,
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };

    let result = service.create_project(input).await;
    assert!(
        matches!(result, Err(ProjectError::Validation(_))),
        "empty name should fail validation"
    );

    // Test: Name too long should fail
    let input = CreateProject {
        name: "a".repeat(101),
        user_id,
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };

    let result = service.create_project(input).await;
    assert!(
        matches!(result, Err(ProjectError::Validation(_))),
        "name too long should fail validation"
    );

    // Test: Invalid characters should fail
    let input = CreateProject {
        name: "invalid name!@#".to_string(),
        user_id,
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };

    let result = service.create_project(input).await;
    assert!(
        matches!(result, Err(ProjectError::Validation(_))),
        "invalid characters should fail validation"
    );

    // Test: Negative budget should fail
    let input = CreateProject {
        name: builder.name("project", "valid"),
        user_id,
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: Some(-100.0),
        tags: vec![],
    };

    let result = service.create_project(input).await;
    assert!(
        matches!(result, Err(ProjectError::Validation(_))),
        "negative budget should fail validation"
    );
}

#[tokio::test]
async fn test_service_authorization() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));
    let builder = TestDataBuilder::from_test_name("authorization");

    let owner = db.create_test_user(Uuid::now_v7()).await;
    let other_user = db.create_test_user(Uuid::now_v7()).await;

    let input = CreateProject {
        name: builder.name("project", "owned"),
        user_id: owner,
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };

    let project = service.create_project(input).await.unwrap();

    // Owner can access
    let result = service.get_project_for_user(project.id, owner).await;
    assert!(result.is_ok(), "owner should be able to access project");

    // Other user cannot access
    let result = service.get_project_for_user(project.id, other_user).await;
    assert!(
        matches!(result, Err(ProjectError::Unauthorized(_))),
        "other user should be unauthorized"
    );

    // Owner can update
    let update = UpdateProject {
        description: Some("Updated".to_string()),
        ..Default::default()
    };
    let result = service
        .update_project_for_user(project.id, owner, update.clone())
        .await;
    assert!(result.is_ok(), "owner should be able to update project");

    // Other user cannot update
    let result = service
        .update_project_for_user(project.id, other_user, update)
        .await;
    assert!(
        matches!(result, Err(ProjectError::Unauthorized(_))),
        "other user should not be able to update"
    );
}

// ============================================================================
// Concurrent Operations Tests
// ============================================================================

#[tokio::test]
async fn test_concurrent_creates() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let builder = TestDataBuilder::from_test_name("concurrent");

    let user_id = db.create_test_user(builder.user_id()).await;

    // Spawn multiple concurrent create operations
    let mut handles = vec![];
    for i in 0..5 {
        let repo_clone = PgProjectRepository::new(db.connection());
        let name = builder.name("project", &format!("concurrent-{i}"));

        let handle = tokio::spawn(async move {
            let input = CreateProject {
                name,
                user_id,
                description: String::new(),
                cloud_provider: CloudProvider::Aws,
                region: "us-east-1".to_string(),
                environment: Environment::Development,
                budget_limit: None,
                tags: vec![],
            };

            repo_clone.create(input).await
        });

        handles.push(handle);
    }

    // Wait for all to complete
    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // All should succeed
    assert_eq!(results.len(), 5);
    for result in results {
        assert!(result.is_ok(), "concurrent create should succeed");
    }

    // Verify all were created
    let filter = ProjectFilter {
        user_id: Some(user_id),
        ..Default::default()
    };
    let all_projects = repo.list(filter).await.unwrap();
    assert_eq!(all_projects.len(), 5, "all projects should be created");
}

// ============================================================================
// Free Tier Limit Tests
// ============================================================================

#[tokio::test]
async fn test_free_tier_can_create_3_projects() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));
    let builder = TestDataBuilder::from_test_name("free_tier_3_projects");

    let user_id = db.create_test_user(builder.user_id()).await;

    // Create 3 projects (the free tier limit)
    for i in 0..3 {
        let input = CreateProject {
            name: builder.name("project", &format!("project-{i}")),
            user_id,
            description: String::new(),
            cloud_provider: CloudProvider::Aws,
            region: "us-east-1".to_string(),
            environment: Environment::Development,
            budget_limit: None,
            tags: vec![],
        };

        let result = service.create_project(input).await;
        assert!(result.is_ok(), "Should be able to create project {i}");
    }
}

#[tokio::test]
async fn test_free_tier_cannot_create_4th_project() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));
    let builder = TestDataBuilder::from_test_name("free_tier_4th_project");

    let user_id = db.create_test_user(builder.user_id()).await;

    // Create 3 projects
    for i in 0..3 {
        let input = CreateProject {
            name: builder.name("project", &format!("project-{i}")),
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

    // Try to create 4th project
    let input = CreateProject {
        name: builder.name("project", "project-4"),
        user_id,
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };

    let result = service.create_project(input).await;
    assert!(
        matches!(result, Err(ProjectError::Validation(_))),
        "Should not be able to create 4th project"
    );

    if let Err(ProjectError::Validation(msg)) = result {
        assert!(
            msg.contains("Free tier limit"),
            "Error message should mention free tier limit"
        );
    }
}

#[tokio::test]
async fn test_free_tier_limit_per_user() {
    let db = TestDatabase::new().await;
    let repo = PgProjectRepository::new(db.connection());
    let service = ProjectService::new(repo, Arc::new(NoopProjectPublisher));
    let builder = TestDataBuilder::from_test_name("free_tier_per_user");

    let user1 = db.create_test_user(Uuid::now_v7()).await;
    let user2 = db.create_test_user(Uuid::now_v7()).await;

    // User 1 creates 3 projects
    for i in 0..3 {
        let input = CreateProject {
            name: builder.name("project", &format!("user1-{i}")),
            user_id: user1,
            description: String::new(),
            cloud_provider: CloudProvider::Aws,
            region: "us-east-1".to_string(),
            environment: Environment::Development,
            budget_limit: None,
            tags: vec![],
        };

        service.create_project(input).await.unwrap();
    }

    // User 2 should still be able to create projects
    let input = CreateProject {
        name: builder.name("project", "user2-0"),
        user_id: user2,
        description: String::new(),
        cloud_provider: CloudProvider::Aws,
        region: "us-east-1".to_string(),
        environment: Environment::Development,
        budget_limit: None,
        tags: vec![],
    };

    let result = service.create_project(input).await;
    assert!(
        result.is_ok(),
        "User 2 should be able to create projects even though User 1 is at the limit"
    );
}
