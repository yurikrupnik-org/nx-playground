//! UUID path parameter extractor with automatic validation.

use crate::errors::AppError;
use axum::{
    extract::{FromRequestParts, Path},
    http::request::Parts,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

/// Extractor for UUID path parameters.
///
/// Automatically parses and validates UUID from path parameters,
/// returning a proper error response if invalid.
///
/// # Example
/// ```ignore
/// use axum::Router;
/// use axum::routing::get;
/// use axum_helpers::extractors::UuidPath;
///
/// async fn get_user(UuidPath(id): UuidPath) -> String {
///     format!("User ID: {}", id)
/// }
///
/// let app = Router::new().route("/users/:id", get(get_user));
/// ```
pub struct UuidPath(pub Uuid);

impl<S> FromRequestParts<S> for UuidPath
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(id) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|e| e.into_response())?;

        match Uuid::parse_str(&id) {
            Ok(uuid) => Ok(UuidPath(uuid)),
            Err(_) => Err(AppError::BadRequest(format!("Invalid UUID: {id}")).into_response()),
        }
    }
}
