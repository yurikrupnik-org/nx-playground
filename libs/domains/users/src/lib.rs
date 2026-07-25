//! Users Domain
//!
//! This module provides a complete domain implementation for user management.
//!
//! # Features
//!
//! - User CRUD operations
//! - IdP-backed identity (WorkOS/OIDC `subject` linking + JIT provisioning)
//! - Email verification
//! - Role-based access control
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
//! │  Handlers   │  ← HTTP endpoints
//! └──────┬──────┘
//!        │
//! ┌──────▼──────┐
//! │   Service   │  ← Business logic, validation, JIT provisioning
//! └──────┬──────┘
//!        │
//! ┌──────▼──────┐
//! │ Repository  │  ← Data access (trait + implementations)
//! └──────┬──────┘
//!        │
//! ┌──────▼──────┐
//! │   Models    │  ← Entities, DTOs, enums
//! └─────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! use domain_users::{
//!     handlers,
//!     repository::InMemoryUserRepository,
//!     service::UserService,
//! };
//!
//! // Create repository and service
//! let repository = InMemoryUserRepository::new();
//! let service = UserService::new(repository);
//!
//! // Create Axum router
//! let router = handlers::router(service);
//! ```

pub mod error;
pub mod handlers;
pub mod models;
pub mod postgres_repository_impl;
pub mod repository;
pub mod service;

// Re-export commonly used types
pub use error::{UserError, UserResult};
pub use handlers::ApiDoc;
pub use models::{CreateUser, Role, UpdateUser, User, UserFilter, UserResponse};
pub use postgres_repository_impl::PostgresUserRepository;
pub use repository::{InMemoryUserRepository, UserRepository};
pub use service::UserService;
