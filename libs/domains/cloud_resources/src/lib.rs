//! Cloud Resources Domain
//!
//! This module provides a complete domain implementation for managing cloud resources.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
//! │  Handlers   │  ← HTTP/gRPC endpoints
//! └──────┬──────┘
//!        │
//! ┌──────▼──────┐
//! │   Service   │  ← Business logic, validation
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
//! use domain_cloud_resources::{
//!     handlers,
//!     postgres::PgCloudResourceRepository,
//!     service::CloudResourceService,
//! };
//! use sea_orm::Database;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create database connection
//! let db = Database::connect("postgres://...").await?;
//!
//! // Create repository and service
//! let repository = PgCloudResourceRepository::new(db);
//! let service = CloudResourceService::new(repository);
//!
//! // Create Axum router
//! let router = handlers::router(service);
//! # Ok(())
//! # }
//! ```

pub mod entity;
pub mod error;
pub mod handlers;
pub mod models;
/// Cluster-observed, read-only inventory (Crossplane `CloudInventory`).
#[cfg(feature = "k8s")]
pub mod observed;
pub mod postgres;
pub mod repository;
pub mod service;

// Re-export commonly used types
pub use error::{CloudResourceError, CloudResourceResult};
pub use handlers::ApiDoc;
pub use models::{
    CloudResource, CloudResourceFilter, CreateCloudResource, ResourceStatus, ResourceType, Tag,
    UpdateCloudResource,
};
pub use postgres::PgCloudResourceRepository;
pub use repository::CloudResourceRepository;
pub use service::CloudResourceService;
