//! HTTP middleware module.
//!
//! This module provides HTTP-level middleware for:
//! - CORS configuration
//! - CSRF protection
//! - Security headers
//!
//! # Example
//!
//! ```ignore
//! use axum_helpers::http::{create_cors_layer, security_headers};
//!
//! let app = Router::new()
//!     .layer(axum::middleware::from_fn(security_headers))
//!     .layer(create_cors_layer(origin));
//! ```

pub mod cors;
pub mod csrf;
pub mod security;

// Re-export commonly used functions
pub use cors::{create_cors_layer, create_permissive_cors_layer};
pub use csrf::{CsrfConfig, csrf_protect};
pub use security::security_headers;
