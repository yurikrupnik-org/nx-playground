use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Errors raised while authenticating a request or talking to the identity provider.
///
/// Variants map to HTTP statuses via [`AuthError::status`]; client-facing bodies are
/// intentionally terse (details are logged, not returned).
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing or malformed credentials")]
    MissingCredentials,
    #[error("invalid username or password")]
    InvalidCredentials,
    #[error("invalid token: {0}")]
    InvalidToken(String),
    #[error("token signing key not found (kid={0})")]
    UnknownKey(String),
    #[error("organization context required but absent")]
    OrgRequired,
    #[error("session not found or expired")]
    SessionInvalid,
    #[error("identity provider error: {0}")]
    Provider(String),
    #[error("session store unavailable")]
    StoreUnavailable,
    #[error("internal auth error: {0}")]
    Internal(String),
}

impl AuthError {
    /// HTTP status this error maps to.
    pub fn status(&self) -> StatusCode {
        match self {
            AuthError::MissingCredentials
            | AuthError::InvalidCredentials
            | AuthError::InvalidToken(_)
            | AuthError::UnknownKey(_)
            | AuthError::OrgRequired
            | AuthError::SessionInvalid => StatusCode::UNAUTHORIZED,
            AuthError::Provider(_) => StatusCode::BAD_GATEWAY,
            // Fail closed: an unavailable session store denies, never bypasses.
            AuthError::StoreUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            AuthError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let status = self.status();
        // Log the full error; never leak token/session detail to the client.
        tracing::debug!(error = %self, %status, "auth rejected request");
        let body = match status {
            StatusCode::UNAUTHORIZED => "unauthorized",
            StatusCode::SERVICE_UNAVAILABLE => "authentication temporarily unavailable",
            StatusCode::BAD_GATEWAY => "identity provider error",
            _ => "internal error",
        };
        (status, body).into_response()
    }
}

/// Crate result alias. The error type is a defaulted parameter so call sites stay
/// short (`Result<T>`) while precise error types remain available (`Result<T, E>`).
pub type Result<T, E = AuthError> = std::result::Result<T, E>;
