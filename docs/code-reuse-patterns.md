# Code Reuse Patterns for Services and Repositories

This document shows patterns to reduce code duplication across domains without compromising modularity.

## Problem: Code Duplication

When you have 5+ domains, you'll see repeated patterns:

```rust
// In EVERY domain's service.rs
pub async fn create_X(&self, input: CreateX) -> Result<X> {
    self.validate_create(&input)?;  // ← Same validation pattern
    self.repository.create(input).await  // ← Same repository call
}

pub async fn get_X(&self, id: Uuid) -> Result<X> {
    self.repository
        .get_by_id(id).await?
        .ok_or(XError::NotFound(id))  // ← Same pattern
}
```

```rust
// In EVERY domain's postgres.rs
async fn create(&self, input: CreateX) -> Result<X> {
    sqlx::query_as(/* SQL */)  // ← Similar patterns
        .bind(/* params */)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| /* error mapping */)  // ← Same error mapping
}
```

## Solutions Overview

| Pattern | Use Case | Pros | Cons |
|---------|----------|------|------|
| **Generic Base Repository** | Standard CRUD | - Eliminate 80% of boilerplate<br>- Type-safe | - Requires trait bounds<br>- Complex generics |
| **Service Composition** | Shared behavior | - Flexible<br>- Clear dependencies | - More setup code |
| **Shared Utilities** | Cross-cutting concerns | - Simple<br>- Reusable | - Not domain-specific |
| **Macros** | Generate boilerplate | - DRY<br>- Compile-time | - Hard to debug<br>- Complex |

---

## Pattern 1: Generic Base Repository

### Current Duplication

```rust
// libs/domains/projects/src/postgres.rs
impl ProjectRepository for PgProjectRepository {
    async fn get_by_id(&self, id: Uuid) -> Result<Option<Project>> {
        sqlx::query_as("SELECT * FROM projects WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ProjectError::Internal(e.to_string()))
    }

    async fn delete(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM projects WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

// libs/domains/users/src/postgres.rs
impl UserRepository for PgUserRepository {
    async fn get_by_id(&self, id: Uuid) -> Result<Option<User>> {
        sqlx::query_as("SELECT * FROM users WHERE id = $1")  // ← Same!
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| UserError::Internal(e.to_string()))
    }

    async fn delete(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM users WHERE id = $1")  // ← Same!
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
```

### Solution: Base Repository

Create `libs/shared/repository/src/lib.rs`:

```rust
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// Generic CRUD repository for entities with UUID primary keys
pub struct BasePgRepository<T, E>
where
    T: FromRow<'static, sqlx::postgres::PgRow> + Send + Unpin,
    E: From<sqlx::Error> + Send,
{
    pool: PgPool,
    table_name: &'static str,
    _phantom: std::marker::PhantomData<(T, E)>,
}

impl<T, E> BasePgRepository<T, E>
where
    T: FromRow<'static, sqlx::postgres::PgRow> + Send + Unpin,
    E: From<sqlx::Error> + Send,
{
    pub fn new(pool: PgPool, table_name: &'static str) -> Self {
        Self {
            pool,
            table_name,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Generic get by ID - works for any table with UUID id
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<T>, E> {
        let query = format!("SELECT * FROM {} WHERE id = $1", self.table_name);

        sqlx::query_as(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(E::from)
    }

    /// Generic delete by ID
    pub async fn delete(&self, id: Uuid) -> Result<bool, E> {
        let query = format!("DELETE FROM {} WHERE id = $1", self.table_name);

        let result = sqlx::query(&query)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(E::from)?;

        Ok(result.rows_affected() > 0)
    }

    /// Generic list with pagination
    pub async fn list(&self, limit: i64, offset: i64) -> Result<Vec<T>, E> {
        let query = format!(
            "SELECT * FROM {} ORDER BY created_at DESC LIMIT $1 OFFSET $2",
            self.table_name
        );

        sqlx::query_as(&query)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(E::from)
    }

    /// Count total rows
    pub async fn count(&self) -> Result<i64, E> {
        let query = format!("SELECT COUNT(*) FROM {}", self.table_name);

        sqlx::query_scalar(&query)
            .fetch_one(&self.pool)
            .await
            .map_err(E::from)
    }
}
```

### Usage in Domains

```rust
// libs/domains/projects/src/postgres.rs
use shared_repository::BasePgRepository;

pub struct PgProjectRepository {
    base: BasePgRepository<ProjectRow, ProjectError>,
    pool: PgPool,  // Keep for complex queries
}

impl PgProjectRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            base: BasePgRepository::new(pool.clone(), "projects"),
            pool,
        }
    }
}

#[async_trait]
impl ProjectRepository for PgProjectRepository {
    async fn get_by_id(&self, id: Uuid) -> ProjectResult<Option<Project>> {
        // Delegate to base! ✨
        let row = self.base.get_by_id(id).await?;
        row.map(|r| r.try_into()).transpose()
    }

    async fn delete(&self, id: Uuid) -> ProjectResult<bool> {
        // Delegate to base! ✨
        self.base.delete(id).await
    }

    async fn list(&self, filter: ProjectFilter) -> ProjectResult<Vec<Project>> {
        if filter.has_custom_filters() {
            // Use custom query for complex filters
            self.list_with_filters(filter).await
        } else {
            // Use base for simple pagination! ✨
            let rows = self.base.list(filter.limit as i64, filter.offset as i64).await?;
            rows.into_iter().map(|r| r.try_into()).collect()
        }
    }

    async fn create(&self, input: CreateProject) -> ProjectResult<Project> {
        // Domain-specific logic remains custom
        // ...
    }
}
```

**Benefits:**

- Eliminates ~50% of repository boilerplate
- Type-safe (compiler enforces correct types)
- Each domain still controls complex queries

---

## Pattern 2: Generic Service Operations

### Current Duplication

```rust
// Every service has this pattern:
pub async fn get_X(&self, id: Uuid) -> Result<X> {
    self.repository
        .get_by_id(id).await?
        .ok_or(XError::NotFound(id))
}

pub async fn delete_X(&self, id: Uuid) -> Result<()> {
    let deleted = self.repository.delete(id).await?;
    if !deleted {
        return Err(XError::NotFound(id));
    }
    Ok(())
}
```

### Solution: Service Trait with Default Implementations

Create `libs/shared/service/src/lib.rs`:

```rust
use async_trait::async_trait;
use uuid::Uuid;

/// Common CRUD operations with default implementations
#[async_trait]
pub trait CrudService<T, CreateDto, UpdateDto, R, E>
where
    R: CrudRepository<T, CreateDto, UpdateDto, E> + Send + Sync,
    E: From<NotFoundError> + Send,
    T: Send,
    CreateDto: Send,
    UpdateDto: Send,
{
    fn repository(&self) -> &R;

    /// Default implementation - domains can override if needed
    async fn get(&self, id: Uuid) -> Result<T, E> {
        self.repository()
            .get_by_id(id)
            .await?
            .ok_or_else(|| NotFoundError(id).into())
    }

    /// Default implementation
    async fn delete(&self, id: Uuid) -> Result<(), E> {
        let deleted = self.repository().delete(id).await?;
        if !deleted {
            return Err(NotFoundError(id).into());
        }
        Ok(())
    }

    /// Domains must implement (has validation logic)
    async fn create(&self, input: CreateDto) -> Result<T, E>;

    /// Domains must implement (has validation logic)
    async fn update(&self, id: Uuid, input: UpdateDto) -> Result<T, E>;
}

pub struct NotFoundError(pub Uuid);
```

### Usage in Domains

```rust
// libs/domains/projects/src/service.rs
use shared_service::CrudService;

pub struct ProjectService<R: ProjectRepository> {
    repository: Arc<R>,
}

impl<R: ProjectRepository> CrudService<
    Project,
    CreateProject,
    UpdateProject,
    R,
    ProjectError,
> for ProjectService<R> {
    fn repository(&self) -> &R {
        &self.repository
    }

    // get() and delete() are FREE! Use default implementation ✨

    // Only implement domain-specific logic
    async fn create(&self, input: CreateProject) -> ProjectResult<Project> {
        self.validate_create(&input)?;
        self.repository.create(input).await
    }

    async fn update(&self, id: Uuid, input: UpdateProject) -> ProjectResult<Project> {
        self.validate_update(&input)?;
        self.repository.update(id, input).await
    }
}

impl<R: ProjectRepository> ProjectService<R> {
    // Domain-specific methods
    pub async fn activate_project(&self, id: Uuid) -> ProjectResult<Project> {
        // Custom logic...
    }
}
```

**Benefits:**

- Eliminate repetitive get/delete implementations
- Domains only implement what's unique
- Still allows custom behavior

---

## Pattern 3: Shared Validation Utilities

### Current Duplication

```rust
// In projects/service.rs
fn validate_name(&self, name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(Error::Validation("Name cannot be empty"));
    }
    if name.len() > 100 {
        return Err(Error::Validation("Name too long"));
    }
    Ok(())
}

// In users/service.rs
fn validate_name(&self, name: &str) -> Result<()> {
    if name.trim().is_empty() {  // ← Same!
        return Err(Error::Validation("Name cannot be empty"));
    }
    if name.len() > 100 {  // ← Same!
        return Err(Error::Validation("Name too long"));
    }
    Ok(())
}
```

### Solution: Shared Validators

Create `libs/shared/validators/src/lib.rs`:

```rust
#[derive(Debug, Clone)]
pub struct StringValidator {
    min_length: Option<usize>,
    max_length: Option<usize>,
    pattern: Option<regex::Regex>,
    trim_whitespace: bool,
}

impl StringValidator {
    pub fn new() -> Self {
        Self {
            min_length: None,
            max_length: None,
            pattern: None,
            trim_whitespace: true,
        }
    }

    pub fn min_length(mut self, min: usize) -> Self {
        self.min_length = Some(min);
        self
    }

    pub fn max_length(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }

    pub fn pattern(mut self, pattern: &str) -> Self {
        self.pattern = Some(regex::Regex::new(pattern).unwrap());
        self
    }

    pub fn validate(&self, value: &str, field_name: &str) -> Result<(), String> {
        let value = if self.trim_whitespace {
            value.trim()
        } else {
            value
        };

        if let Some(min) = self.min_length {
            if value.len() < min {
                return Err(format!("{} must be at least {} characters", field_name, min));
            }
        }

        if let Some(max) = self.max_length {
            if value.len() > max {
                return Err(format!("{} cannot exceed {} characters", field_name, max));
            }
        }

        if let Some(ref pattern) = self.pattern {
            if !pattern.is_match(value) {
                return Err(format!("{} has invalid format", field_name));
            }
        }

        Ok(())
    }
}

// Pre-configured validators
pub mod validators {
    use super::*;

    pub fn name() -> StringValidator {
        StringValidator::new()
            .min_length(1)
            .max_length(100)
    }

    pub fn email() -> StringValidator {
        StringValidator::new()
            .max_length(255)
            .pattern(r"^[^\s@]+@[^\s@]+\.[^\s@]+$")
    }

    pub fn slug() -> StringValidator {
        StringValidator::new()
            .min_length(1)
            .max_length(100)
            .pattern(r"^[a-z0-9-_]+$")
    }
}
```

### Usage in Domains

```rust
// libs/domains/projects/src/service.rs
use shared_validators::validators;

impl<R: ProjectRepository> ProjectService<R> {
    fn validate_create(&self, input: &CreateProject) -> ProjectResult<()> {
        // Reuse shared validator! ✨
        validators::name()
            .validate(&input.name, "Project name")
            .map_err(|e| ProjectError::Validation(e))?;

        // Domain-specific validation
        if input.budget_limit.unwrap_or(0.0) < 0.0 {
            return Err(ProjectError::Validation("Budget cannot be negative"));
        }

        Ok(())
    }
}

// libs/domains/users/src/service.rs
impl<R: UserRepository> UserService<R> {
    fn validate_create(&self, input: &CreateUser) -> UserResult<()> {
        // Same validator, different domain! ✨
        validators::name()
            .validate(&input.name, "User name")
            .map_err(|e| UserError::Validation(e))?;

        validators::email()
            .validate(&input.email, "Email")
            .map_err(|e| UserError::Validation(e))?;

        Ok(())
    }
}
```

---

## Pattern 4: Error Mapping Utilities

### Current Duplication

```rust
// Every postgres.rs has similar error mapping
.map_err(|e| match e {
    sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => {
        XError::Duplicate(...)
    }
    sqlx::Error::RowNotFound => XError::NotFound(...),
    _ => XError::Internal(e.to_string()),
})
```

### Solution: Generic Error Mapper

Create `libs/shared/db-utils/src/lib.rs`:

```rust
use sqlx::Error as SqlxError;

pub trait DbErrorMapper: Sized {
    fn from_unique_violation(constraint: String) -> Self;
    fn from_not_found() -> Self;
    fn from_internal(msg: String) -> Self;
}

pub fn map_sqlx_error<E: DbErrorMapper>(err: SqlxError) -> E {
    match err {
        SqlxError::Database(ref db_err) if db_err.is_unique_violation() => {
            let constraint = db_err
                .constraint()
                .unwrap_or("unknown")
                .to_string();
            E::from_unique_violation(constraint)
        }
        SqlxError::RowNotFound => E::from_not_found(),
        _ => E::from_internal(err.to_string()),
    }
}
```

### Usage

```rust
// libs/domains/projects/src/error.rs
impl DbErrorMapper for ProjectError {
    fn from_unique_violation(constraint: String) -> Self {
        if constraint.contains("name") {
            ProjectError::DuplicateName("Name already exists".to_string())
        } else {
            ProjectError::Internal(format!("Constraint violation: {}", constraint))
        }
    }

    fn from_not_found() -> Self {
        ProjectError::Internal("Not found".to_string())
    }

    fn from_internal(msg: String) -> Self {
        ProjectError::Internal(msg)
    }
}

// libs/domains/projects/src/postgres.rs
use shared_db_utils::map_sqlx_error;

async fn create(&self, input: CreateProject) -> ProjectResult<Project> {
    sqlx::query_as(/* ... */)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?  // ✨ One liner!
}
```

---

## Pattern 5: Macros for Boilerplate

### Current Duplication

```rust
// Every domain has nearly identical handler functions
async fn get_by_id<R: XRepository>(
    State(service): State<Arc<XService<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<X>, XError> {
    let item = service.get(id).await?;
    Ok(Json(item))
}
```

### Solution: Macro for Standard Handlers

Create `libs/shared/handler-macros/src/lib.rs`:

```rust
#[macro_export]
macro_rules! crud_handlers {
    ($service:ty, $entity:ty, $create_dto:ty, $update_dto:ty) => {
        pub async fn list(
            State(service): State<Arc<$service>>,
            Query(filter): Query<ListFilter>,
        ) -> Result<Json<Vec<$entity>>, Error> {
            let items = service.list(filter).await?;
            Ok(Json(items))
        }

        pub async fn get_by_id(
            State(service): State<Arc<$service>>,
            Path(id): Path<Uuid>,
        ) -> Result<Json<$entity>, Error> {
            let item = service.get(id).await?;
            Ok(Json(item))
        }

        pub async fn create(
            State(service): State<Arc<$service>>,
            Json(input): Json<$create_dto>,
        ) -> Result<(StatusCode, Json<$entity>), Error> {
            let item = service.create(input).await?;
            Ok((StatusCode::CREATED, Json(item)))
        }

        pub async fn update(
            State(service): State<Arc<$service>>,
            Path(id): Path<Uuid>,
            Json(input): Json<$update_dto>,
        ) -> Result<Json<$entity>, Error> {
            let item = service.update(id, input).await?;
            Ok(Json(item))
        }

        pub async fn delete(
            State(service): State<Arc<$service>>,
            Path(id): Path<Uuid>,
        ) -> Result<StatusCode, Error> {
            service.delete(id).await?;
            Ok(StatusCode::NO_CONTENT)
        }
    };
}
```

### Usage

```rust
// libs/domains/projects/src/handlers.rs
use handler_macros::crud_handlers;

// Generate all standard handlers in one line! ✨
crud_handlers!(
    ProjectService<impl ProjectRepository>,
    Project,
    CreateProject,
    UpdateProject
);

// Only write custom handlers
pub async fn activate(
    State(service): State<Arc<ProjectService<impl ProjectRepository>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Project>, ProjectError> {
    let project = service.activate_project(id).await?;
    Ok(Json(project))
}

pub fn router<R: ProjectRepository + 'static>(
    service: ProjectService<R>,
) -> Router {
    Router::new()
        .route("/", get(list).post(create))  // ✨ Generated!
        .route("/{id}", get(get_by_id).put(update).delete(delete))  // ✨ Generated!
        .route("/{id}/activate", post(activate))  // Custom
        .with_state(Arc::new(service))
}
```

---

## Recommended Approach: Mix and Match

Use a combination based on what makes sense:

### Phase 1: Low Effort, High Value

```text
libs/shared/
├── validators/      ← Start here (easiest win)
└── db-utils/        ← Error mapping
```

**Savings**: ~20% code reduction with minimal complexity

### Phase 2: Medium Effort, Medium Value

```text
libs/shared/
├── validators/
├── db-utils/
└── repository/      ← Generic base repository
```

**Savings**: ~40% code reduction, some generics complexity

### Phase 3: High Effort, High Value

```text
libs/shared/
├── validators/
├── db-utils/
├── repository/
├── service/         ← Generic service traits
└── handler-macros/  ← Code generation
```

**Savings**: ~60% code reduction, significant complexity

---

## Directory Structure Example

```text
libs/
├── domains/              # Domain modules (unchanged)
│   ├── projects/
│   ├── users/
│   └── tasks/
│
└── shared/              # ✨ New: Shared utilities
    ├── repository/      # Generic CRUD repository
    │   ├── Cargo.toml
    │   └── src/lib.rs
    │
    ├── service/         # Service trait with defaults
    │   ├── Cargo.toml
    │   └── src/lib.rs
    │
    ├── validators/      # Validation utilities
    │   ├── Cargo.toml
    │   └── src/lib.rs
    │
    ├── db-utils/        # Database helpers
    │   ├── Cargo.toml
    │   └── src/lib.rs
    │
    └── handler-macros/  # Code generation macros
        ├── Cargo.toml
        └── src/lib.rs
```

---

## Tradeoffs Summary

| Approach | Code Reduction | Complexity | Maintainability | Testability |
|----------|---------------|------------|-----------------|-------------|
| **None** (current) | 0% | ⭐ Low | ⭐⭐⭐ Easy | ⭐⭐⭐ Easy |
| **Validators** | 15-20% | ⭐ Low | ⭐⭐⭐ Easy | ⭐⭐⭐ Easy |
| **Base Repository** | 30-40% | ⭐⭐ Medium | ⭐⭐ Medium | ⭐⭐ Medium |
| **Service Traits** | 40-50% | ⭐⭐⭐ High | ⭐⭐ Medium | ⭐⭐ Medium |
| **Macros** | 50-60% | ⭐⭐⭐⭐ Very High | ⭐ Hard | ⭐ Hard |

## Key Principles

1. **Start Simple**: Begin with validators and utilities
2. **Measure Impact**: Only add complexity if duplication is >5 domains
3. **Keep Domains Independent**: Shared code is a dependency - be careful
4. **Override When Needed**: All patterns should allow domain-specific behavior
5. **Document Well**: Complex generics need good documentation

## When NOT to Share

❌ **Don't share:**

- Domain-specific business rules
- Validation that differs between domains
- Complex queries unique to one domain
- Domain events (keep these local)

✅ **Do share:**

- Generic CRUD operations
- Common validation patterns
- Error mapping utilities
- Database connection logic

The goal is **reducing boilerplate**, not **creating abstractions for everything**.
