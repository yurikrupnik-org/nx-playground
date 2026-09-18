# E2E Test Plan for Zerg API

> **Status: plan only.** Nothing below exists yet — `apps/zerg/api/tests/` is
> not a directory. The repo's implemented e2e suite is the browser-level
> Playwright project for the todo vertical, `apps/todo/e2e` (`just e2e`); see
> `docs/TESTING_GUIDE.md` § E2E Tests.

## Overview

End-to-End (E2E) tests verify the **entire application** from HTTP request to database and back. These tests ensure all components work together correctly in production-like conditions.

## What E2E Tests Cover

Unlike domain handler tests, E2E tests include:
- ✅ **Full application routing** (`/api/projects`, `/api/cloud-resources`, etc.)
- ✅ **Authentication middleware** (JWT validation, token extraction)
- ✅ **Authorization** (user can only access their own resources)
- ✅ **Global middleware** (CORS, logging, rate limiting, error handling)
- ✅ **Cross-domain interactions** (creating project + cloud resource together)
- ✅ **Exactly what users experience** in production

## Test Structure

This plan lives in `docs/`; the tests it describes belong in the crate:

```
apps/zerg/api/tests/
├── common/
│   └── mod.rs                ← Test helpers (start_app, create_auth_token, etc.)
└── e2e_test.rs               ← E2E tests
```

## Implementation Checklist

### Phase 1: Basic Setup
- [ ] Add test dependencies to `apps/zerg/api/Cargo.toml`
  ```toml
  [dev-dependencies]
  test-utils = { workspace = true }
  http-body-util = "0.1"
  tower = { workspace = true }
  serde_json = { workspace = true }
  ```

- [ ] Create `tests/common/mod.rs` with helpers:
  ```rust
  pub async fn start_test_app() -> Router {
      let db = TestDatabase::new().await;
      // Build full app with all domains
      create_app(db.connection()).await
  }

  pub fn create_test_jwt(user_id: Uuid) -> String {
      // Generate valid JWT for testing
  }
  ```

### Phase 2: Critical User Journeys

#### Journey 1: New User Onboarding
```rust
#[tokio::test]
async fn e2e_new_user_creates_first_project() {
    let app = start_test_app().await;

    // 1. Register user
    let response = app
        .post("/api/auth/register")
        .json(&json!({
            "email": "test@example.com",
            "password": "password123"
        }))
        .send()
        .await;
    assert_eq!(response.status(), 201);

    // 2. Login
    let response = app
        .post("/api/auth/login")
        .json(&json!({
            "email": "test@example.com",
            "password": "password123"
        }))
        .send()
        .await;
    let token: String = response.json().await["token"];

    // 3. Create project
    let response = app
        .post("/api/projects")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "my-first-project",
            "cloud_provider": "aws",
            "region": "us-east-1"
        }))
        .send()
        .await;
    assert_eq!(response.status(), 201);

    // 4. Verify project exists
    let response = app
        .get("/api/projects")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await;
    let projects: Vec<Project> = response.json().await;
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "my-first-project");
}
```

#### Journey 2: Free Tier Limit
```rust
#[tokio::test]
async fn e2e_free_tier_limit_enforced() {
    let app = start_test_app().await;
    let token = create_test_jwt(Uuid::new_v4());

    // Create 3 projects (limit)
    for i in 0..3 {
        let response = app
            .post("/api/projects")
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "name": format!("project-{}", i),
                "cloud_provider": "aws",
                "region": "us-east-1"
            }))
            .send()
            .await;
        assert_eq!(response.status(), 201);
    }

    // 4th should fail
    let response = app
        .post("/api/projects")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "project-4",
            "cloud_provider": "aws",
            "region": "us-east-1"
        }))
        .send()
        .await;
    assert_eq!(response.status(), 400);
    assert!(response.text().await.contains("Free tier limit"));
}
```

#### Journey 3: Authorization
```rust
#[tokio::test]
async fn e2e_users_cannot_access_others_projects() {
    let app = start_test_app().await;

    let user1_id = Uuid::new_v4();
    let user2_id = Uuid::new_v4();
    let user1_token = create_test_jwt(user1_id);
    let user2_token = create_test_jwt(user2_id);

    // User 1 creates project
    let response = app
        .post("/api/projects")
        .header("Authorization", format!("Bearer {}", user1_token))
        .json(&json!({
            "name": "user1-project",
            "cloud_provider": "aws",
            "region": "us-east-1"
        }))
        .send()
        .await;
    let project_id = response.json().await["id"];

    // User 2 tries to access it
    let response = app
        .get(format!("/api/projects/{}", project_id))
        .header("Authorization", format!("Bearer {}", user2_token))
        .send()
        .await;
    assert_eq!(response.status(), 403);  // Forbidden
}
```

#### Journey 4: Cross-Domain Operations
```rust
#[tokio::test]
async fn e2e_create_project_and_cloud_resources() {
    let app = start_test_app().await;
    let token = create_test_jwt(Uuid::new_v4());

    // 1. Create project
    let response = app
        .post("/api/projects")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "my-project",
            "cloud_provider": "aws",
            "region": "us-east-1"
        }))
        .send()
        .await;
    let project_id = response.json().await["id"];

    // 2. Create cloud resource for that project
    let response = app
        .post("/api/cloud-resources")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "my-ec2-instance",
            "project_id": project_id,
            "resource_type": "compute",
            "region": "us-east-1"
        }))
        .send()
        .await;
    assert_eq!(response.status(), 201);

    // 3. List project's resources
    let response = app
        .get(format!("/api/cloud-resources?project_id={}", project_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await;
    let resources: Vec<CloudResource> = response.json().await;
    assert_eq!(resources.len(), 1);
}
```

### Phase 3: Error Scenarios

```rust
#[tokio::test]
async fn e2e_missing_auth_token_returns_401() {
    let app = start_test_app().await;

    let response = app
        .post("/api/projects")
        .json(&json!({"name": "test"}))
        .send()
        .await;
    assert_eq!(response.status(), 401);  // Unauthorized
}

#[tokio::test]
async fn e2e_invalid_json_returns_400() {
    let app = start_test_app().await;
    let token = create_test_jwt(Uuid::new_v4());

    let response = app
        .post("/api/projects")
        .header("Authorization", format!("Bearer {}", token))
        .header("content-type", "application/json")
        .body("{invalid json")
        .send()
        .await;
    assert_eq!(response.status(), 400);
}
```

## Test Execution Strategy

### Local Development
```bash
# Run only E2E tests
cargo test -p zerg_api --test e2e_test

# Run with output
cargo test -p zerg_api --test e2e_test -- --nocapture

# Run specific test
cargo test -p zerg_api --test e2e_test e2e_free_tier_limit_enforced
```

### CI/CD Pipeline
```yaml
# .github/workflows/test.yml
- name: Run E2E Tests
  run: |
    cargo test -p zerg_api --test e2e_test
    # Run last to catch integration issues
```

## Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Total E2E test time | < 30 seconds | For ~10 tests |
| Single test time | < 5 seconds | Including DB setup |
| Container startup | < 2 seconds | Postgres 18 |

## When to Add E2E Tests

Add E2E tests for:
- ✅ Critical user journeys (signup, login, create resources)
- ✅ Cross-domain operations
- ✅ Authentication/authorization flows
- ✅ Payment/billing flows
- ✅ Complex business rules that span domains

**Don't add E2E tests for:**
- ❌ Simple CRUD (handler tests cover this)
- ❌ Validation logic (unit tests cover this)
- ❌ Database queries (integration tests cover this)

## Maintenance

- **Review monthly**: Remove flaky tests, update for new features
- **Keep focused**: Each test should verify ONE user journey
- **Use helpers**: Abstract common operations (login, create project)
- **Parallel execution**: Use `--test-threads` carefully (database isolation)

## Future Enhancements

- [ ] Visual regression tests (screenshots of UI responses)
- [ ] Performance benchmarks (response time tracking)
- [ ] Contract testing with external APIs
- [ ] Chaos engineering (random failures)
