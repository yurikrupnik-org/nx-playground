//! Distributed rate limiting using Redis sliding window counters.
//!
//! This module provides a Redis-backed rate limiter that works correctly across
//! multiple replicas behind an HPA. It uses a sliding window counter algorithm
//! implemented as an atomic Lua script for O(1) Redis operations per check.
//!
//! # Example
//!
//! ```ignore
//! use axum_helpers::rate_limit::{RateLimiter, RateLimitConfig};
//! use redis::aio::ConnectionManager;
//!
//! let config = RateLimitConfig::from_env();
//! let limiter = RateLimiter::new(redis_conn, config);
//!
//! // Apply as middleware
//! let app = Router::new()
//!     .route("/api/resource", get(handler))
//!     .layer(axum::middleware::from_fn_with_state(
//!         limiter,
//!         rate_limit_middleware,
//!     ));
//! ```

mod middleware;

pub use middleware::rate_limit_middleware;

use redis::aio::ConnectionManager;

/// Per-route rate limit configuration inserted into request extensions.
///
/// Routes with no `RateLimitTier` in their extensions are exempt from rate limiting.
/// Apply via `axum::Extension(tier)` on individual sub-routers.
#[derive(Clone, Debug)]
pub struct RateLimitTier {
    /// Name used in Redis key prefix for counter isolation (e.g. "standard", "vector")
    pub name: String,
    /// Max requests per window for this tier
    pub requests_per_window: u64,
    /// Window duration in seconds
    pub window_secs: u64,
}

/// Configuration for the rate limiter.
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    /// Maximum requests allowed per window
    pub requests_per_window: u64,
    /// Window duration in seconds
    pub window_secs: u64,
    /// Whether rate limiting is enabled
    pub enabled: bool,
}

impl RateLimitConfig {
    /// Load rate limit configuration from environment variables with defaults.
    pub fn from_env() -> Self {
        let requests_per_window = std::env::var("RATE_LIMIT_REQUESTS_PER_WINDOW")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);

        let window_secs = std::env::var("RATE_LIMIT_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let enabled = std::env::var("RATE_LIMIT_ENABLED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(true);

        Self {
            requests_per_window,
            window_secs,
            enabled,
        }
    }
}

/// Sliding window counter Lua script.
///
/// Atomically increments the current window counter and computes a weighted
/// estimate using the previous window's count. Returns `[allowed, remaining, reset_at]`.
///
/// If over the limit, the increment is undone so counts stay accurate.
const SLIDING_WINDOW_SCRIPT: &str = r#"
local curr_key = KEYS[1]
local prev_key = KEYS[2]
local limit = tonumber(ARGV[1])
local window = tonumber(ARGV[2])
local now = tonumber(ARGV[3])

local window_start = math.floor(now / window) * window
local elapsed = now - window_start
local weight = (window - elapsed) / window

local prev_count = tonumber(redis.call('GET', prev_key) or '0') or 0
local curr_count = tonumber(redis.call('INCR', curr_key)) or 0

if curr_count == 1 then
    redis.call('EXPIRE', curr_key, window * 2)
end

local estimated = math.floor(prev_count * weight) + curr_count
local remaining = math.max(0, limit - estimated)
local reset_at = window_start + window

if estimated > limit then
    redis.call('DECR', curr_key)
    return {0, remaining, reset_at}
end

return {1, remaining, reset_at}
"#;

impl RateLimitTier {
    pub fn new(name: impl Into<String>, requests_per_window: u64, window_secs: u64) -> Self {
        Self {
            name: name.into(),
            requests_per_window,
            window_secs,
        }
    }
}

/// Redis-backed distributed rate limiter.
///
/// Uses a sliding window counter algorithm for near-perfect accuracy with O(1)
/// Redis operations per check. Safe to use across multiple replicas.
#[derive(Clone)]
pub struct RateLimiter {
    redis: ConnectionManager,
    config: RateLimitConfig,
}

/// Result of a rate limit check.
pub struct RateLimitResult {
    /// Whether the request is allowed
    pub allowed: bool,
    /// Remaining requests in the current window
    pub remaining: u64,
    /// Unix timestamp when the current window resets
    pub reset_at: u64,
}

impl RateLimiter {
    /// Create a new rate limiter with the given Redis connection and config.
    pub fn new(redis: ConnectionManager, config: RateLimitConfig) -> Self {
        Self { redis, config }
    }

    /// Check if a request identified by `key` should be allowed.
    ///
    /// Returns `Ok(RateLimitResult)` on success, or `Err` if Redis is unavailable.
    pub async fn check(&self, key: &str) -> Result<RateLimitResult, redis::RedisError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_secs();

        let window = self.config.window_secs;
        let current_window = (now / window) * window;
        let previous_window = current_window - window;

        let curr_key = format!("rl:{}:{}", key, current_window);
        let prev_key = format!("rl:{}:{}", key, previous_window);

        let mut conn = self.redis.clone();
        let result: Vec<i64> = redis::Script::new(SLIDING_WINDOW_SCRIPT)
            .key(curr_key)
            .key(prev_key)
            .arg(self.config.requests_per_window)
            .arg(window)
            .arg(now)
            .invoke_async(&mut conn)
            .await?;

        Ok(RateLimitResult {
            allowed: result[0] == 1,
            remaining: result[1] as u64,
            reset_at: result[2] as u64,
        })
    }

    /// Check if a request should be allowed using per-call overrides.
    ///
    /// Uses the provided `tier_name` as a key prefix, and the given
    /// `requests_per_window` and `window_secs` instead of `self.config`.
    pub async fn check_with_config(
        &self,
        key: &str,
        tier_name: &str,
        requests_per_window: u64,
        window_secs: u64,
    ) -> Result<RateLimitResult, redis::RedisError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_secs();

        let current_window = (now / window_secs) * window_secs;
        let previous_window = current_window - window_secs;

        let curr_key = format!("rl:{}:{}:{}", tier_name, key, current_window);
        let prev_key = format!("rl:{}:{}:{}", tier_name, key, previous_window);

        let mut conn = self.redis.clone();
        let result: Vec<i64> = redis::Script::new(SLIDING_WINDOW_SCRIPT)
            .key(curr_key)
            .key(prev_key)
            .arg(requests_per_window)
            .arg(window_secs)
            .arg(now)
            .invoke_async(&mut conn)
            .await?;

        Ok(RateLimitResult {
            allowed: result[0] == 1,
            remaining: result[1] as u64,
            reset_at: result[2] as u64,
        })
    }

    /// Whether rate limiting is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// The configured limit per window.
    pub fn limit(&self) -> u64 {
        self.config.requests_per_window
    }
}
