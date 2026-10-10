//! Root OpenAPI document for `todo_api`.
//!
//! `main` hand-builds its router (the tonic routes are merged *after* the HTTP
//! layers, which `axum_helpers::create_router` cannot express), so this document
//! is assembled here and served by a single `GET /api-docs/openapi.json` route
//! rather than by the shared router helper and its four doc UIs.
//!
//! `/api/events/sse` and `/api/events/ws` are deliberately NOT annotated: they
//! are open-ended streams, not request/response operations, so they have no
//! useful representation in a generated client or CLI command tree.

use utoipa::OpenApi;

/// The whole HTTP surface of `todo_api`: the `domain_todo` REST routes nested at
/// their mount path, plus the app-level stack profiles endpoint.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Todo API",
        version = "1.0.0",
        description = "Todo vertical: CRUD + lifecycle over Postgres, plus frontend stack profiles. \
                       The same service is also reachable as gRPC (todo.v1.TodoService) and as \
                       SSE/WebSocket streams on this port; only the request/response routes are \
                       described here."
    ),
    servers((url = "/api", description = "todo-api mount path")),
    paths(crate::stacks::list_stacks),
    nest((path = "/todos", api = domain_todo::handlers::TodoApiDoc)),
    components(schemas(axum_helpers::ErrorResponse, crate::stacks::StackProfile)),
    tags(
        (name = "todos", description = "Todo CRUD + lifecycle operations"),
        (name = "stacks", description = "Frontend stack profiles (reference data, cheapest-first)")
    )
)]
pub struct ApiDoc;

/// `GET /api-docs/openapi.json` — the document itself, for doc UIs and for the
/// `x` CLI when it is pointed at a running process instead of `docs/openapi/`.
///
/// The served document and the committed one must be byte-identical in
/// substance: `x --spec http://host/api-docs/openapi.json` builds the same
/// command tree as the embedded copy, so any divergence here is a CLI that
/// addresses routes this process does not serve. Path keys need no fixing —
/// the nested handlers are annotated `path = ""` — but `nest` also appends the
/// nested document's own `servers` (`/api/todos`, where *it* is mounted
/// standalone), which is not an alternative origin for this document.
pub async fn serve_document() -> axum::Json<utoipa::openapi::OpenApi> {
    let mut doc = ApiDoc::openapi();
    doc.servers = doc.servers.map(|mut servers| {
        servers.truncate(1);
        servers
    });
    axum::Json(doc)
}

#[cfg(test)]
mod tests {
    use test_utils::openapi;
    use utoipa::OpenApi;

    fn document() -> serde_json::Value {
        openapi::normalized(
            serde_json::to_value(super::ApiDoc::openapi()).expect("serialize todo openapi doc"),
        )
    }

    /// Regenerates the committed OpenAPI v1 document. Same convention as the
    /// ts-rs `export_bindings_*` tests: running the suite keeps
    /// `docs/openapi/todo.v1.json` in sync with the handler annotations.
    #[test]
    fn export_openapi_todo_v1() {
        openapi::export(
            serde_json::to_value(super::ApiDoc::openapi()).expect("serialize todo openapi doc"),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../docs/openapi/todo.v1.json"),
        );
    }

    /// The `x` CLI derives its command tree from this document alone.
    #[test]
    fn document_satisfies_cli_invariants() {
        openapi::assert_invariants(&document());
    }

    /// Nesting must contribute the domain's routes under the mount path without
    /// importing its `info`/`servers`, and must not leave the trailing slash
    /// that `utoipa`'s string-concat nesting produces for a `path = "/"` route
    /// (axum serves `/api/todos`, and 0.8 does not redirect the slashed form).
    #[test]
    fn nested_todo_routes_land_on_the_routes_axum_serves() {
        let doc = document();

        assert_eq!(doc["info"]["title"], "Todo API");
        assert_eq!(doc["servers"][0]["url"], "/api");
        assert_eq!(doc["servers"].as_array().unwrap().len(), 1);

        for key in [
            "/todos",
            "/todos/{id}",
            "/todos/{id}/complete",
            "/todos/{id}/uncomplete",
            "/stacks",
        ] {
            assert!(doc["paths"].get(key).is_some(), "{key} is described");
        }
        assert!(
            doc["paths"].get("/todos/").is_none(),
            "no trailing-slash route: axum 0.8 does not redirect it"
        );
    }

    /// The doc endpoint is this binary's only OpenAPI surface (no
    /// `create_router`, no Swagger UI), so a wrong route path would leave the
    /// document unreachable at runtime with every test above still green.
    #[tokio::test]
    async fn doc_endpoint_serves_the_document() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let response = axum::Router::new()
            .route(
                "/api-docs/openapi.json",
                axum::routing::get(super::serve_document),
            )
            .oneshot(
                Request::builder()
                    .uri("/api-docs/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let served: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(served["info"]["title"], "Todo API");
        assert!(served["paths"].get("/stacks").is_some());
    }
}
