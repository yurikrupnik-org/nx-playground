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

/// Upstream gRPC failure → HTTP. The service's `Status` code carries the intent;
/// the message is logged, never forwarded (it may describe internals).
impl From<tonic::Status> for ApiError {
    fn from(e: tonic::Status) -> Self {
        use tonic::Code;
        let (status, message) = match e.code() {
            Code::NotFound => (StatusCode::NOT_FOUND, "not found"),
            Code::InvalidArgument | Code::FailedPrecondition | Code::OutOfRange => {
                (StatusCode::BAD_REQUEST, "invalid request")
            }
            Code::PermissionDenied => (StatusCode::FORBIDDEN, "forbidden"),
            Code::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Code::Unavailable | Code::DeadlineExceeded => {
                (StatusCode::SERVICE_UNAVAILABLE, "service unavailable")
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
        };
        tracing::warn!(code = ?e.code(), detail = e.message(), "upstream gRPC error");
        Self::new(status, message)
    }
}

/// A proto message that does not decode into a domain type is our bug, not the
/// caller's - the two sides are generated from the same `.proto`.
impl From<contract_tasks::ConversionError> for ApiError {
    fn from(e: contract_tasks::ConversionError) -> Self {
        tracing::error!(error = %e, "proto conversion failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;
