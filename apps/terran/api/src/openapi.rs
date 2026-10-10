use utoipa::Modify;
use utoipa::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};

/// OpenAPI document for the terran API. Served (Swagger/ReDoc/RapiDoc/Scalar) by
/// `axum_helpers::server::create_router`. Handler `#[utoipa::path]` annotations are
/// registered in `paths(...)`; the `/api` base path matches the router nesting.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Terran API",
        version = "0.1.0",
        description = "B2B multi-tenant observability platform API"
    ),
    servers((url = "/api", description = "API base path")),
    paths(
        crate::auth::login,
        crate::auth::callback,
        crate::auth::logout,
        crate::auth::password_login,
        crate::auth::me,
        crate::assets::list_assets,
        crate::assets::get_asset,
        crate::assets::list_assets_by_user,
        crate::assets::create_asset,
        crate::inventory::list_cloud_resources,
        crate::inventory::get_cloud_resource,
    ),
    components(schemas(
        crate::db::CloudAsset,
        crate::assets::CreateAsset,
        crate::auth::PasswordLogin,
        domain_cloud_resources::observed::ObservedCloudResource,
        domain_cloud_resources::ResourceType,
        domain_cloud_resources::ResourceStatus,
        domain_cloud_resources::Tag,
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "auth", description = "Authentication / BFF session endpoints"),
        (name = "assets", description = "Tenant-scoped cloud inventory"),
        (name = "cloud-resources", description = "Read-only cluster-observed cloud inventory")
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
                SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("terran_session"))),
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
            serde_json::to_value(super::ApiDoc::openapi()).expect("serialize terran openapi doc"),
        )
    }

    /// Regenerates the committed OpenAPI v1 document. Same convention as the
    /// ts-rs `export_bindings_*` tests: running the suite keeps
    /// `docs/openapi/terran.v1.json` in sync with the handler annotations.
    #[test]
    fn export_openapi_terran_v1() {
        openapi::export(
            serde_json::to_value(super::ApiDoc::openapi()).expect("serialize terran openapi doc"),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../docs/openapi/terran.v1.json"),
        );
    }

    /// The `x` CLI derives its command tree from this document alone.
    #[test]
    fn document_satisfies_cli_invariants() {
        openapi::assert_invariants(&document());
    }
}
