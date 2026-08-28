use domain_projects::ApiResource;
use utoipa::Modify;
use utoipa::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};

#[derive(OpenApi)]
#[openapi(
    components(
        schemas(axum_helpers::ErrorResponse, crate::api::auth::PasswordLogin)
    ),
    info(
        title = "Zerg API",
        version = "0.1.0",
        description = "API for managing tasks, projects, cloud resources, and users"
    ),
    servers(
        (url = "/api", description = "API base path")
    ),
    paths(
        crate::api::auth::login,
        crate::api::auth::callback,
        crate::api::auth::password_login,
        crate::api::auth::logout,
        crate::api::auth::me,
    ),
    nest(
        (path = "/tasks", api = crate::api::tasks::TasksApiDoc),
        (path = domain_projects::entity::Model::URL, api = domain_projects::ApiDoc),
        (path = "/users", api = domain_users::ApiDoc),
        (path = "/cloud-resources", api = domain_cloud_resources::ApiDoc),
        (path = "/vector", api = domain_vector::VectorApiDoc)
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "auth", description = "Authentication / BFF session endpoints (WorkOS AuthKit)")
    )
)]
pub struct ApiDoc;

/// Registers the opaque session-cookie auth scheme referenced by guarded routes.
/// The cookie name mirrors the `SESSION_COOKIE_NAME` default (`config::Config`).
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "session_cookie",
                SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("zerg_session"))),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use utoipa::OpenApi;

    /// Nested docs (tasks v1 etc.) carry their own `info`/`servers`; nesting
    /// must merge only paths/schemas/tags, never the root document's metadata.
    #[test]
    fn nested_v1_docs_do_not_override_root_metadata() {
        let doc = serde_json::to_value(super::ApiDoc::openapi()).unwrap();
        assert_eq!(doc["info"]["title"], "Zerg API");
        assert_eq!(doc["servers"][0]["url"], "/api");
        assert!(doc["paths"].get("/tasks").is_some(), "tasks routes nested");
    }
}
