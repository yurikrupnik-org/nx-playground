//! Users Domain
//!
//! This module provides a complete domain implementation for user management.
//!
//! # Features
//!
//! - User CRUD operations
//! - Password hashing with Argon2
//! - Email verification
//! - Role-based access control
//! - Login/authentication
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
//! │  Handlers   │  ← HTTP endpoints
//! └──────┬──────┘
//!        │
//! ┌──────▼──────┐
//! │   Service   │  ← Business logic, password hashing, validation
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

pub mod auth_handlers;
pub mod error;
pub mod handlers;
pub mod models;
pub mod oauth;
pub mod password;
pub mod postgres_repository_impl;
pub mod repository;
pub mod service;

// Re-export commonly used types
pub use auth_handlers::AuthApiDoc;
pub use error::{UserError, UserResult};
pub use handlers::ApiDoc;
pub use models::{CreateUser, LoginRequest, Role, UpdateUser, User, UserFilter, UserResponse};
pub use oauth::{AccountLinkingService, OAuthStateManager, PostgresOAuthAccountRepository};
pub use postgres_repository_impl::PostgresUserRepository;
pub use repository::{InMemoryUserRepository, UserRepository};
pub use service::UserService;
