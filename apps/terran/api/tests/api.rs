//! Integration tests against the live dev stack (Postgres `terran`, Keycloak `:8088`,
//! Redis `:6379`). `#[ignore]`d so CI without the stack stays green. Run with:
//!   cargo test --package terran_api --test api -- --ignored

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use core_config::{Environment, server::ServerConfig};
use terran_api::config::Config;
use terran_api::db::{self, NewAsset};

const DATABASE_URL: &str = "postgres://myuser:mypassword@localhost:5432/terran?sslmode=disable";
/// Non-superuser role so RLS actually applies (created by `just db-fresh terran`).
const APP_ROLE_URL: &str = "postgres://terran_app:terran_app@localhost:5432/terran?sslmode=disable";
const ISSUER: &str = "http://localhost:8088/realms/terran";

fn test_config() -> Config {
    Config {
        server: ServerConfig {
            host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port: 0,
        },
        environment: Environment::Development,
        database_url: DATABASE_URL.to_string(),
        redis_url: "redis://127.0.0.1:6379".to_string(),
        oidc_issuer: ISSUER.to_string(),
        oidc_client_id: "terran-api".to_string(),
        oidc_client_secret: "local-dev-secret".to_string(),
        oidc_scopes: "openid profile email organization".to_string(),
        oidc_audience: None,
        redirect_base_url: "http://localhost:8081".to_string(),
        frontend_url: "http://localhost:3001".to_string(),
        cookie_name: "terran_session".to_string(),
        cookie_secure: false,
        session_ttl_secs: 3600,
    }
}

fn sample_asset(provider: &str, external_id: &str) -> NewAsset {
    NewAsset {
        provider: provider.to_string(),
        external_id: external_id.to_string(),
        name: "test-asset".to_string(),
        asset_type: "ec2/m5.large".to_string(),
        region: "us-east-1".to_string(),
        status: "active".to_string(),
        monthly_cost: 12.5,
        metadata: json!({"k": "v"}),
    }
}

async fn fetch_keycloak_token() -> String {
    let body = reqwest::Url::parse_with_params(
        "http://form.local/",
        &[
            ("grant_type", "password"),
            ("client_id", "terran-api"),
            ("client_secret", "local-dev-secret"),
            ("username", "test@terran.dev"),
            ("password", "test"),
        ],
    )
    .unwrap()
    .query()
    .unwrap()
    .to_string();
    let resp = reqwest::Client::new()
        .post(format!("{ISSUER}/protocol/openid-connect/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .expect("keycloak reachable (is the stack up?)");
    let json: Value = resp.json().await.unwrap();
    json["access_token"]
        .as_str()
        .expect("access_token")
        .to_string()
}

#[tokio::test]
#[ignore = "requires live Postgres terran db"]
async fn cross_tenant_isolation_at_repo_layer() {
    let db = db::connect(DATABASE_URL).await.expect("connect terran db");

    // Two distinct tenants with one asset each.
    let suffix = Uuid::new_v4().simple().to_string();
    let org_a = db::upsert_org(&db, &format!("itest-a-{suffix}"), "Org A")
        .await
        .unwrap();
    let org_b = db::upsert_org(&db, &format!("itest-b-{suffix}"), "Org B")
        .await
        .unwrap();
    let asset_a = db::create_asset(
        &db,
        org_a,
        None,
        &sample_asset("aws", &format!("a-{suffix}")),
    )
    .await
    .unwrap();
    let asset_b = db::create_asset(
        &db,
        org_b,
        None,
        &sample_asset("gcp", &format!("b-{suffix}")),
    )
    .await
    .unwrap();

    // Org A sees only its own asset.
    let listed_a = db::list_assets_for_org(&db, org_a).await.unwrap();
    assert!(
        listed_a.iter().any(|x| x.id == asset_a.id),
        "A sees its asset"
    );
    assert!(
        !listed_a.iter().any(|x| x.id == asset_b.id),
        "A must NOT see B's asset"
    );

    // Org A cannot fetch Org B's asset by id (no cross-tenant read / IDOR).
    assert!(
        db::get_asset_for_org(&db, org_a, asset_b.id)
            .await
            .unwrap()
            .is_none(),
        "A must not read B's asset by id"
    );
    // But can fetch its own.
    assert!(
        db::get_asset_for_org(&db, org_a, asset_a.id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
#[ignore = "requires live terran db + the terran_app role (just db-fresh terran)"]
async fn rls_blocks_cross_tenant_under_app_role() {
    // Connect as the non-superuser app role so RLS is the enforcer — not a WHERE clause.
    let db = db::connect(APP_ROLE_URL)
        .await
        .expect("connect as terran_app");

    let suffix = Uuid::new_v4().simple().to_string();
    let org_a = db::upsert_org(&db, &format!("rls-a-{suffix}"), "RLS A")
        .await
        .unwrap();
    let org_b = db::upsert_org(&db, &format!("rls-b-{suffix}"), "RLS B")
        .await
        .unwrap();
    let asset_a = db::create_asset(
        &db,
        org_a,
        None,
        &sample_asset("aws", &format!("ra-{suffix}")),
    )
    .await
    .unwrap();
    db::create_asset(
        &db,
        org_b,
        None,
        &sample_asset("gcp", &format!("rb-{suffix}")),
    )
    .await
    .unwrap();

    // With app.org_id = A, an *unfiltered* scan returns only A's rows: RLS hides B.
    let mut tx = db.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.org_id', $1, true)")
        .bind(org_a.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let rows: Vec<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as("SELECT id, org_id FROM cloud_assets")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(!rows.is_empty(), "A's own rows are visible");
    assert!(
        rows.iter().any(|(id, _)| *id == asset_a.id),
        "A sees its asset"
    );
    assert!(
        rows.iter().all(|(_, org)| *org == org_a),
        "an unfiltered scan must not leak other tenants — RLS enforces isolation"
    );

    // With no app.org_id set, RLS yields nothing (fails closed at the data layer).
    let mut tx = db.begin().await.unwrap();
    let none: Vec<(uuid::Uuid,)> = sqlx::query_as("SELECT id FROM cloud_assets")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(none.is_empty(), "no app.org_id -> zero rows under RLS");
}

#[tokio::test]
#[ignore = "requires live Keycloak + Postgres + Redis"]
async fn bearer_path_through_router() {
    let state = terran_api::build_state(test_config())
        .await
        .expect("build state");
    // create_router (inside build_app) requires CORS_ALLOWED_ORIGIN.
    unsafe { std::env::set_var("CORS_ALLOWED_ORIGIN", "http://localhost:3001") };
    let app = terran_api::build_app(state).await.expect("build app");
    let token = fetch_keycloak_token().await;

    // Unauthenticated request to a guarded route is rejected.
    let unauth = app
        .clone()
        .oneshot(Request::get("/api/assets").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED, "no creds -> 401");

    // Authenticated create.
    let ext = Uuid::new_v4().simple().to_string();
    let create_body = json!({
        "provider": "aws", "external_id": ext, "name": "bearer-asset",
        "asset_type": "ec2/t3.micro", "region": "eu-west-1",
        "status": "active", "monthly_cost": 9.0
    });
    let created = app
        .clone()
        .oneshot(
            Request::post("/api/assets")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(create_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        created.status(),
        StatusCode::CREATED,
        "authenticated create -> 201"
    );

    // Authenticated list includes the created asset (same user's tenant).
    let listed = app
        .clone()
        .oneshot(
            Request::get("/api/assets")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(listed.into_body(), usize::MAX)
        .await
        .unwrap();
    let assets: Value = serde_json::from_slice(&bytes).unwrap();
    let found = assets
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["external_id"] == ext);
    assert!(found, "created asset visible to the authenticated tenant");

    // A bad bearer token is rejected.
    let bad = app
        .oneshot(
            Request::get("/api/assets")
                .header(header::AUTHORIZATION, "Bearer not-a-jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        bad.status(),
        StatusCode::UNAUTHORIZED,
        "garbage token -> 401"
    );
}

#[tokio::test]
#[ignore = "requires live Postgres + Redis (build_state)"]
async fn csrf_guards_cookie_authed_writes() {
    let state = terran_api::build_state(test_config())
        .await
        .expect("build state");
    unsafe { std::env::set_var("CORS_ALLOWED_ORIGIN", "http://localhost:3001") };
    let app = terran_api::build_app(state).await.expect("build app");

    // Cookie-authed POST without the double-submit header is rejected by CSRF (403),
    // before auth even runs.
    let no_csrf = app
        .clone()
        .oneshot(
            Request::post("/api/assets")
                .header(header::COOKIE, "terran_csrf=tok; terran_session=whatever")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        no_csrf.status(),
        StatusCode::FORBIDDEN,
        "missing csrf header -> 403"
    );

    // A matching double-submit token clears CSRF; the request then fails auth (bad
    // session), proving CSRF itself did not block a well-formed request.
    let with_csrf = app
        .oneshot(
            Request::post("/api/assets")
                .header(header::COOKIE, "terran_csrf=tok; terran_session=whatever")
                .header("x-csrf-token", "tok")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        with_csrf.status(),
        StatusCode::UNAUTHORIZED,
        "valid double-submit passes csrf; bad session -> 401 from auth"
    );
}

#[tokio::test]
#[ignore = "requires live Keycloak + Postgres + Redis"]
async fn password_login_issues_session() {
    let state = terran_api::build_state(test_config())
        .await
        .expect("build state");
    unsafe { std::env::set_var("CORS_ALLOWED_ORIGIN", "http://localhost:3001") };
    let app = terran_api::build_app(state).await.expect("build app");

    // Wrong password → 401, no session. No CSRF cookie/header is sent, proving the
    // login route is exempt from the double-submit guard (it predates the cookie).
    let bad = app
        .clone()
        .oneshot(
            Request::post("/api/auth/login/password")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"email": "test@terran.dev", "password": "wrong"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        bad.status(),
        StatusCode::UNAUTHORIZED,
        "bad password -> 401"
    );

    // Correct credentials → 204 + Set-Cookie (session + readable csrf).
    let ok = app
        .clone()
        .oneshot(
            Request::post("/api/auth/login/password")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"email": "test@terran.dev", "password": "test"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::NO_CONTENT, "valid login -> 204");

    let cookies: Vec<String> = ok
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|c| c.split(';').next().unwrap_or("").to_string())
        .collect();
    let session_cookie = cookies
        .iter()
        .find(|c| c.starts_with("terran_session="))
        .expect("session cookie set")
        .clone();
    assert!(
        cookies.iter().any(|c| c.starts_with("terran_csrf=")),
        "csrf cookie set alongside the session"
    );

    // The minted session resolves an authenticated principal at /me.
    let me = app
        .oneshot(
            Request::get("/api/auth/me")
                .header(header::COOKIE, session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK, "session authenticates /me");
    let body = axum::body::to_bytes(me.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["email"], "test@terran.dev", "identity from the token");
    assert!(json["org_id"].is_string(), "tenant resolved");
}

/// The OpenAPI document must expose every annotated route + schema, so Swagger UI
/// (`/swagger-ui`) renders endpoints instead of an empty spec. Pure generation — no
/// live stack — so this runs in normal CI (unlike the `#[ignore]`d integration tests).
#[test]
fn openapi_spec_lists_all_endpoints() {
    use utoipa::OpenApi;

    let spec = serde_json::to_value(terran_api::openapi::ApiDoc::openapi()).unwrap();

    let paths = spec["paths"].as_object().expect("paths object");
    for p in [
        "/auth/login",
        "/auth/callback",
        "/auth/logout",
        "/auth/login/password",
        "/auth/me",
        "/assets",
        "/assets/{id}",
        "/assets/by-user/{user_id}",
    ] {
        assert!(paths.contains_key(p), "missing path {p} in OpenAPI spec");
    }

    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("component schemas");
    for s in ["CloudAsset", "CreateAsset", "PasswordLogin"] {
        assert!(
            schemas.contains_key(s),
            "missing schema {s} in OpenAPI spec"
        );
    }

    assert!(
        spec["components"]["securitySchemes"]["session_cookie"].is_object(),
        "session_cookie security scheme must be registered"
    );
}

/// `GET /assets/by-user/{user_id}` must coexist with `/assets/{id}` (overlapping
/// static vs. param segment). Builds the path tree without a live stack — a routing
/// conflict would panic here at registration time.
#[test]
fn asset_routes_register_without_conflict() {
    use axum::routing::get;

    let _router: axum::Router<terran_api::state::AppState> = axum::Router::new()
        .route("/assets", get(terran_api::assets::list_assets))
        .route("/assets/{id}", get(terran_api::assets::get_asset))
        .route(
            "/assets/by-user/{user_id}",
            get(terran_api::assets::list_assets_by_user),
        );
}

#[tokio::test]
#[ignore = "requires live Postgres terran db"]
async fn list_assets_for_user_filters_by_discoverer() {
    let db = db::connect(DATABASE_URL).await.expect("connect terran db");
    let suffix = Uuid::new_v4().simple().to_string();

    let org = db::upsert_org(&db, &format!("itest-byuser-{suffix}"), "Org ByUser")
        .await
        .unwrap();
    let alice = db::upsert_user(
        &db,
        &format!("alice-{suffix}"),
        &format!("alice-{suffix}@terran.dev"),
        "Alice",
    )
    .await
    .unwrap();
    let bob = db::upsert_user(
        &db,
        &format!("bob-{suffix}"),
        &format!("bob-{suffix}@terran.dev"),
        "Bob",
    )
    .await
    .unwrap();

    let alice_asset = db::create_asset(
        &db,
        org,
        Some(alice),
        &sample_asset("aws", &format!("alice-{suffix}")),
    )
    .await
    .unwrap();
    let bob_asset = db::create_asset(
        &db,
        org,
        Some(bob),
        &sample_asset("gcp", &format!("bob-{suffix}")),
    )
    .await
    .unwrap();

    // Filtering by Alice returns only Alice's asset, scoped to the org.
    let alices = db::list_assets_for_user(&db, org, alice).await.unwrap();
    assert!(
        alices.iter().any(|x| x.id == alice_asset.id),
        "alice's asset present"
    );
    assert!(
        !alices.iter().any(|x| x.id == bob_asset.id),
        "bob's asset must be excluded"
    );
    assert!(
        alices.iter().all(|x| x.org_id == org),
        "all results within the caller's org"
    );

    // An unknown user id yields an empty list (no leak), never an error.
    let none = db::list_assets_for_user(&db, org, Uuid::new_v4())
        .await
        .unwrap();
    assert!(none.is_empty(), "unknown user id → no assets");
}
