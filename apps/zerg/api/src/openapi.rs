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
        (path = "/org", api = crate::api::org::OrgApiDoc),
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
pub(crate) struct SecurityAddon;

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
    use test_utils::openapi;
    use utoipa::OpenApi;

    fn document() -> serde_json::Value {
        openapi::normalized(
            serde_json::to_value(super::ApiDoc::openapi()).expect("serialize zerg openapi doc"),
        )
    }

    /// Nested docs (tasks v1 etc.) carry their own `info`/`servers`; nesting
    /// must merge only paths/schemas/tags, never the root document's metadata.
    #[test]
    fn nested_v1_docs_do_not_override_root_metadata() {
        let doc = document();
        assert_eq!(doc["info"]["title"], "Zerg API");
        assert_eq!(doc["servers"][0]["url"], "/api");
        assert!(doc["paths"].get("/tasks").is_some(), "tasks routes nested");
    }

    /// The `/api/org` handlers were annotated but registered nowhere, so they
    /// existed only as dead proc-macro output. Each key here is a route
    /// `crate::api::org::router` actually serves.
    #[test]
    fn org_routes_are_described() {
        let doc = document();
        for key in [
            "/org",
            "/org/members",
            "/org/invitations",
            "/org/invitations/{id}/revoke",
        ] {
            assert!(doc["paths"].get(key).is_some(), "{key} is described");
        }
    }

    /// Regenerates the committed OpenAPI v1 document. Same convention as the
    /// ts-rs `export_bindings_*` tests: running the suite keeps
    /// `docs/openapi/zerg.v1.json` in sync with the handler annotations.
    #[test]
    fn export_openapi_zerg_v1() {
        openapi::export(
            serde_json::to_value(super::ApiDoc::openapi()).expect("serialize zerg openapi doc"),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../docs/openapi/zerg.v1.json"),
        );
    }

    /// The `x` CLI derives its command tree from this document alone.
    #[test]
    fn document_satisfies_cli_invariants() {
        openapi::assert_invariants(&document());
    }
}
