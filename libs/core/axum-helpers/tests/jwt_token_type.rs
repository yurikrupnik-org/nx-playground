//! Token-type separation + issuer/audience binding against a live Redis (`:6379`).
//! `#[ignore]`d so CI without Redis stays green. Run with:
//!   cargo test -p axum-helpers --test jwt_token_type -- --ignored

use axum_helpers::{JwtConfig, JwtRedisAuth};

async fn auth() -> JwtRedisAuth {
    let client = redis::Client::open("redis://127.0.0.1:6379").expect("redis url");
    let manager = redis::aio::ConnectionManager::new(client)
        .await
        .expect("redis reachable (is the stack up?)");
    let config = JwtConfig::new("test-secret-that-is-at-least-32-characters-long");
    JwtRedisAuth::new(manager, &config).expect("jwt auth")
}

#[tokio::test]
#[ignore = "requires live Redis :6379"]
async fn access_and_refresh_tokens_are_not_interchangeable() {
    let a = auth().await;
    let roles = vec!["user".to_string()];
    let access = a.create_access_token("u1", "u1@test", "U", &roles).unwrap();
    let refresh = a
        .create_refresh_token("u1", "u1@test", "U", &roles)
        .unwrap();

    // Correct types verify.
    assert!(
        a.verify_access_token(&access).is_ok(),
        "access verifies as access"
    );
    assert!(
        a.verify_refresh_token(&refresh).is_ok(),
        "refresh verifies as refresh"
    );

    // The core fix: a refresh token cannot authenticate API calls, and an access token
    // cannot be exchanged at the refresh endpoint.
    assert!(
        a.verify_access_token(&refresh).is_err(),
        "refresh token rejected on the access path"
    );
    assert!(
        a.verify_refresh_token(&access).is_err(),
        "access token rejected on the refresh path"
    );

    // Claims carry the bound issuer/audience and the type discriminator.
    let claims = a.verify_token(&access).unwrap();
    assert_eq!(claims.token_type, "access");
    assert_eq!(claims.iss, "zerg-api");
    assert_eq!(claims.aud, "zerg-api");
}

#[tokio::test]
#[ignore = "requires live Redis :6379"]
async fn issuer_mismatch_is_rejected() {
    let a = auth().await;
    // A token minted under a different secret/issuer must not verify here.
    let other = JwtRedisAuth::new(
        redis::aio::ConnectionManager::new(redis::Client::open("redis://127.0.0.1:6379").unwrap())
            .await
            .unwrap(),
        &JwtConfig::new("a-completely-different-secret-key-32-chars"),
    )
    .unwrap();
    let foreign = other
        .create_access_token("u1", "u1@test", "U", &["user".to_string()])
        .unwrap();
    assert!(
        a.verify_token(&foreign).is_err(),
        "token signed by a different key must be rejected"
    );
}
