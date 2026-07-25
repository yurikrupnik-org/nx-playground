use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Unified handler error → HTTP response. Details are logged, not leaked.
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

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!(error = %e, "database error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

impl From<oidc_auth::AuthError> for ApiError {
    fn from(e: oidc_auth::AuthError) -> Self {
        let status = e.status();
        tracing::warn!(error = %e, "auth error in handler");
        Self::new(status, "authentication error")
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;
