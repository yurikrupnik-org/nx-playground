//! Integration tests against the local dev stack (Keycloak `:8088`, Redis `:6379`).
//!
//! These require the running compose stack (`just reset-db` / `just _docker-up`) and
//! are `#[ignore]`d so CI without the stack stays green. Run locally with:
//!   cargo test --package oidc-auth --test integration -- --ignored
#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on a broken fixture is the intended failure mode"
)]

use oidc_auth::{OidcVerifier, RedisSessionStore, SessionRecord, SessionStore, VerifierConfig};

const ISSUER: &str = "http://localhost:8088/realms/terran";
const REDIS_URL: &str = "redis://127.0.0.1:6379";

/// Fetch a real access token from the live Keycloak via the direct-access grant.
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
        .expect("keycloak reachable on :8088 (is the stack up?)");
    assert!(
        resp.status().is_success(),
        "token grant failed: {}",
        resp.status()
    );
    let json: serde_json::Value = resp.json().await.unwrap();
    json["access_token"]
        .as_str()
        .expect("access_token")
        .to_string()
}

#[tokio::test]
#[ignore = "requires live Keycloak on :8088"]
async fn verifies_real_keycloak_token_via_live_jwks() {
    let token = fetch_keycloak_token().await;

    // No seeding: the verifier must fetch the JWKS from the live realm endpoint.
    let verifier = OidcVerifier::new(VerifierConfig::keycloak(ISSUER));
    let identity = verifier.verify(&token).await.expect("verify real token");

    assert!(!identity.subject.is_empty(), "subject present");
    assert_eq!(identity.email.as_deref(), Some("test@terran.dev"));
    assert!(
        identity.has_role("org_admin"),
        "seeded test user has org_admin realm role; got {:?}",
        identity.roles
    );
}

#[tokio::test]
#[ignore = "requires live Keycloak on :8088"]
async fn rejects_tampered_token() {
    let token = fetch_keycloak_token().await;
    // Flip the last char of the signature segment.
    let mut tampered = token.clone();
    let last = tampered.pop().unwrap();
    tampered.push(if last == 'A' { 'B' } else { 'A' });

    let verifier = OidcVerifier::new(VerifierConfig::keycloak(ISSUER));
    assert!(
        verifier.verify(&tampered).await.is_err(),
        "tampered token must be rejected"
    );
}

#[tokio::test]
#[ignore = "requires live Redis on :6379"]
async fn redis_session_lifecycle_and_revocation() {
    let store = RedisSessionStore::connect(REDIS_URL, "terran-test")
        .await
        .expect("redis reachable on :6379 (is the stack up?)");

    let rec = SessionRecord {
        subject: "user-int-1".to_string(),
        org_id: Some("org-int-1".to_string()),
        roles: vec!["member".to_string()],
        email: Some("int@terran.dev".to_string()),
        name: Some("Integration User".to_string()),
        access_token: "at-int".to_string(),
        refresh_token: Some("rt-int".to_string()),
        id_token: None,
        access_expires_at: 0,
        session_expires_at: 0,
    };

    // create + round-trip
    let id = store.create(&rec, 60).await.expect("create");
    let got = store.get(&id).await.expect("get").expect("present");
    assert_eq!(got, rec, "round-tripped record matches");

    // unknown id -> None (not an error)
    assert!(store.get("does-not-exist").await.expect("get").is_none());

    // single delete
    store.delete(&id).await.expect("delete");
    assert!(
        store.get(&id).await.expect("get").is_none(),
        "deleted session gone"
    );

    // bulk revocation for a user
    let a = store.create(&rec, 60).await.expect("create a");
    let b = store.create(&rec, 60).await.expect("create b");
    store
        .delete_all_for_user("user-int-1")
        .await
        .expect("revoke all");
    assert!(
        store.get(&a).await.expect("get").is_none(),
        "session a revoked"
    );
    assert!(
        store.get(&b).await.expect("get").is_none(),
        "session b revoked"
    );
}
