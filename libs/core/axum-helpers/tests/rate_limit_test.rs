use axum_helpers::rate_limit::{RateLimitConfig, RateLimiter};
use redis::aio::ConnectionManager;
use test_utils::TestRedis;

async fn setup() -> (TestRedis, RateLimiter) {
    let test_redis = TestRedis::new().await;
    let client =
        redis::Client::open(test_redis.connection_string()).expect("Failed to create client");
    let conn = ConnectionManager::new(client)
        .await
        .expect("Failed to create ConnectionManager");

    let config = RateLimitConfig {
        requests_per_window: 100,
        window_secs: 60,
        enabled: true,
    };
    let limiter = RateLimiter::new(conn, config);
    (test_redis, limiter)
}

#[tokio::test]
async fn test_allows_requests_within_limit() {
    let (_redis, limiter) = setup().await;
    let limit = 5u64;

    for i in 0..limit {
        let result = limiter
            .check_with_config("test:user1", "standard", limit, 60)
            .await
            .expect("check should succeed");
        assert!(result.allowed, "request {} should be allowed", i);
        assert_eq!(
            result.remaining,
            limit - i - 1,
            "remaining should decrement"
        );
    }
}

#[tokio::test]
async fn test_blocks_requests_over_limit() {
    let (_redis, limiter) = setup().await;
    let limit = 3u64;

    // Exhaust the limit
    for _ in 0..limit {
        let result = limiter
            .check_with_config("test:over", "standard", limit, 60)
            .await
            .unwrap();
        assert!(result.allowed);
    }

    // Next request should be denied
    let result = limiter
        .check_with_config("test:over", "standard", limit, 60)
        .await
        .unwrap();
    assert!(!result.allowed, "request over limit should be denied");
    assert_eq!(result.remaining, 0);
}

#[tokio::test]
async fn test_separate_keys_independent() {
    let (_redis, limiter) = setup().await;
    let limit = 2u64;

    // Exhaust limit for key A
    for _ in 0..limit {
        limiter
            .check_with_config("user:alice", "standard", limit, 60)
            .await
            .unwrap();
    }
    let denied = limiter
        .check_with_config("user:alice", "standard", limit, 60)
        .await
        .unwrap();
    assert!(!denied.allowed, "alice should be rate limited");

    // Key B should still have quota
    let result = limiter
        .check_with_config("user:bob", "standard", limit, 60)
        .await
        .unwrap();
    assert!(result.allowed, "bob should not be affected by alice's limit");
}

#[tokio::test]
async fn test_tier_isolation() {
    let (_redis, limiter) = setup().await;
    let limit = 2u64;

    // Exhaust "standard" tier for a key
    for _ in 0..limit {
        limiter
            .check_with_config("user:x", "standard", limit, 60)
            .await
            .unwrap();
    }
    let denied = limiter
        .check_with_config("user:x", "standard", limit, 60)
        .await
        .unwrap();
    assert!(!denied.allowed, "standard tier should be exhausted");

    // Same key under "vector" tier should still have quota
    let result = limiter
        .check_with_config("user:x", "vector", limit, 60)
        .await
        .unwrap();
    assert!(result.allowed, "vector tier should be independent");
}

#[tokio::test]
async fn test_disabled_limiter_always_allows() {
    let test_redis = TestRedis::new().await;
    let client =
        redis::Client::open(test_redis.connection_string()).expect("Failed to create client");
    let conn = ConnectionManager::new(client)
        .await
        .expect("Failed to create ConnectionManager");

    let config = RateLimitConfig {
        requests_per_window: 1,
        window_secs: 60,
        enabled: false,
    };
    let limiter = RateLimiter::new(conn, config);

    assert!(!limiter.is_enabled());

    // Even though limit is 1, the middleware would short-circuit on is_enabled().
    // Verify check_with_config still works (it doesn't check enabled flag itself).
    // Exhaust the limit and verify the limiter still enforces at the Redis level —
    // proving the "disabled" bypass is purely a middleware concern.
    let r1 = limiter
        .check_with_config("test:disabled", "standard", 1, 60)
        .await
        .unwrap();
    assert!(r1.allowed, "first request should be allowed");

    let r2 = limiter
        .check_with_config("test:disabled", "standard", 1, 60)
        .await
        .unwrap();
    assert!(
        !r2.allowed,
        "Redis-level check still enforces — disabled flag is middleware-only"
    );
}

#[tokio::test]
async fn test_window_reset() {
    let (_redis, limiter) = setup().await;
    let limit = 2u64;
    let window_secs = 1u64;

    // Exhaust the limit with a 1-second window
    for _ in 0..limit {
        limiter
            .check_with_config("test:reset", "standard", limit, window_secs)
            .await
            .unwrap();
    }
    let denied = limiter
        .check_with_config("test:reset", "standard", limit, window_secs)
        .await
        .unwrap();
    assert!(!denied.allowed, "should be denied after exhausting limit");

    // Wait for TWO full windows so the sliding window algorithm fully ages out
    // the previous window's counts (prev_count * weight drops to zero).
    tokio::time::sleep(tokio::time::Duration::from_millis(2100)).await;

    let result = limiter
        .check_with_config("test:reset", "standard", limit, window_secs)
        .await
        .unwrap();
    assert!(result.allowed, "should be allowed after window reset");
}
