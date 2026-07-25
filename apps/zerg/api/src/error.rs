//! Handler error type for the BFF auth endpoints (mirrors `apps/terran/api`).
//! Details are logged, not leaked; domain routers keep their own error mapping.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Unified auth-handler error → HTTP response.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    pub fn new(status: StatusCode, message: &'static str) -> Self {
        Self { status, message }
    }

    pub fn bad_request(message: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

impl From<oidc_auth::AuthError> for ApiError {
    fn from(e: oidc_auth::AuthError) -> Self {
        let status = e.status();
        tracing::warn!(error = %e, "auth error in handler");
        Self::new(status, "authentication error")
    }
}

impl From<domain_users::error::UserError> for ApiError {
    fn from(e: domain_users::error::UserError) -> Self {
        tracing::error!(error = %e, "user provisioning error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;
