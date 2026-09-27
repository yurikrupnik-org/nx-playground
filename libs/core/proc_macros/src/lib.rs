// Re-export proc macros when their features are enabled
#[cfg(feature = "selectable_fields")]
pub use selectable_fields::SelectableFields;

#[cfg(feature = "api_resource")]
pub use api_resource::ApiResource;

#[cfg(feature = "sea_orm_resource")]
pub use sea_orm_resource::SeaOrmResource;

/// Trait for REST API resource metadata.
///
/// This trait provides constants for resource URLs, database collection names,
/// and API documentation tags. It is typically derived using the `ApiResource` macro.
///
/// # Examples
///
/// The derive (feature `api_resource`) generates exactly this impl; its own
/// examples live in the `api_resource` crate. Written by hand it needs no feature:
///
/// ```
/// use core_proc_macros::ApiResource;
///
/// pub struct User;
///
/// impl ApiResource for User {
///     const URL: &'static str = "/user";
///     const COLLECTION: &'static str = "users";
///     const TAG: &'static str = "Users";
/// }
///
/// assert_eq!(User::URL, "/user");
/// assert_eq!(User::COLLECTION, "users");
/// ```
pub trait ApiResource {
    /// The base URL path for this resource (e.g., "/user")
    const URL: &'static str;
    /// The database collection or table name (e.g., "users")
    const COLLECTION: &'static str;
    /// The API documentation tag (e.g., "Users")
    const TAG: &'static str;
}
