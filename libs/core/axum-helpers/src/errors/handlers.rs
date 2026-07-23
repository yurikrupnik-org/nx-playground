use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{ErrorCode, ErrorResponse};

/// Handler for 404 Not Found errors.
///
/// This can be used as a fallback handler in your router.
pub async fn not_found() -> Response {
    let body = Json(ErrorResponse {
        code: ErrorCode::NotFound.code(),
        error: ErrorCode::NotFound.as_str().to_string(),
        message: "The requested resource was not found".to_string(),
        details: None,
    });

    (StatusCode::NOT_FOUND, body).into_response()
}

/// Handler for 405 Method Not Allowed errors.
pub async fn method_not_allowed() -> Response {
    let body = Json(ErrorResponse {
        code: ErrorCode::MethodNotAllowed.code(),
        error: ErrorCode::MethodNotAllowed.as_str().to_string(),
        message: "The HTTP method is not allowed for this resource".to_string(),
        details: None,
    });

    (StatusCode::METHOD_NOT_ALLOWED, body).into_response()
}
