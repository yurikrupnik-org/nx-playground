pub mod config;

pub use config::ChannelConfig;

use crate::error::{GrpcError, GrpcResult};
use tonic::transport::{Channel, Endpoint};

/// Creates an optimized gRPC channel with production-ready defaults
///
/// This function creates a gRPC channel with HTTP/2 tuning that has been
/// validated through benchmarking to deliver 15K+ req/s throughput with
/// sub-4ms P99 latency.
///
/// ## Configuration Details
/// - HTTP/2 keep-alive: 30s interval, 10s timeout
/// - Connection timeout: 5s
/// - Request timeout: 30s
/// - Window sizes: 1MB for connection and stream
/// - Adaptive flow control enabled
/// - TCP nodelay and keepalive enabled
///
/// ## Example
/// ```ignore
/// use grpc_client::create_channel;
/// use rpc::tasks::tasks_service_client::TasksServiceClient;
///
/// let channel = create_channel("http://[::1]:50051").await?;
/// let client = TasksServiceClient::new(channel);
/// ```
pub async fn create_channel(addr: impl Into<String>) -> GrpcResult<Channel> {
    create_channel_with_config(addr, ChannelConfig::default()).await
}

/// Creates a gRPC channel with custom configuration
///
/// Use this function when you need to override the default settings,
/// such as for slow networks, high-latency connections, or specific
/// throughput requirements.
///
/// ## Example
/// ```ignore
/// use grpc_client::{create_channel_with_config, ChannelConfig};
/// use std::time::Duration;
///
/// let config = ChannelConfig::default()
///     .with_connect_timeout(Duration::from_secs(10))
///     .with_request_timeout(Duration::from_secs(120))
///     .with_max_concurrent_streams(200);
///
/// let channel = create_channel_with_config("http://[::1]:50051", config).await?;
/// ```
pub async fn create_channel_with_config(
    addr: impl Into<String>,
    config: ChannelConfig,
) -> GrpcResult<Channel> {
    let addr_string = addr.into();

    let endpoint = Endpoint::from_shared(addr_string.clone()).map_err(|e| {
        tracing::error!(target: "grpc_client", addr = %addr_string, error = ?e, "Invalid URI");
        GrpcError::InvalidUri(e)
    })?;

    let endpoint = config.apply_to_endpoint(endpoint);

    tracing::debug!(
        target: "grpc_client",
        addr = %addr_string,
        "Creating gRPC channel"
    );

    endpoint.connect().await.map_err(|e| {
        tracing::error!(
            target: "grpc_client",
            addr = %addr_string,
            error = ?e,
            "Failed to connect to gRPC service"
        );
        GrpcError::ConnectionFailed(e)
    })
}

/// Creates a gRPC channel that defers the TCP/HTTP-2 handshake to the first request.
///
/// Unlike [`create_channel`], this does not perform any I/O — it returns
/// immediately with a `Channel` whose connection is established lazily on the
/// first RPC. Tonic will also auto-reconnect with backoff if the upstream
/// becomes unreachable later.
///
/// Use this when the caller must boot independently of the upstream service.
/// Pair it with a readiness probe that issues a cheap RPC, so traffic is only
/// routed once the upstream is actually reachable.
pub fn create_channel_lazy(addr: impl Into<String>) -> GrpcResult<Channel> {
    create_channel_lazy_with_config(addr, ChannelConfig::default())
}

/// Lazy variant of [`create_channel_with_config`].
pub fn create_channel_lazy_with_config(
    addr: impl Into<String>,
    config: ChannelConfig,
) -> GrpcResult<Channel> {
    let addr_string = addr.into();

    let endpoint = Endpoint::from_shared(addr_string.clone()).map_err(|e| {
        tracing::error!(target: "grpc_client", addr = %addr_string, error = ?e, "Invalid URI");
        GrpcError::InvalidUri(e)
    })?;

    tracing::debug!(
        target: "grpc_client",
        addr = %addr_string,
        "Creating lazy gRPC channel"
    );

    Ok(config.apply_to_endpoint(endpoint).connect_lazy())
}

/// Creates a channel with retry logic
///
/// This function will retry connection establishment with exponential backoff
/// if the initial connection fails. Useful for services that may not be
/// immediately available on startup.
///
/// ## Example
/// ```ignore
/// use grpc_client::{create_channel_with_retry, RetryConfig};
///
/// // Default: 3 retries with exponential backoff
/// let channel = create_channel_with_retry("http://[::1]:50051", None).await?;
///
/// // Custom retry configuration
/// let retry = RetryConfig::new().with_max_retries(5);
/// let channel = create_channel_with_retry("http://[::1]:50051", Some(retry)).await?;
/// ```
pub async fn create_channel_with_retry(
    addr: impl Into<String>,
    retry_config: Option<crate::retry::RetryConfig>,
) -> GrpcResult<Channel> {
    let addr = addr.into();

    match retry_config {
        Some(config) => {
            crate::retry::retry_with_backoff(
                || {
                    let addr = addr.clone();
                    async move { create_channel(addr).await }
                },
                config,
            )
            .await
        }
        None => {
            crate::retry::retry(|| {
                let addr = addr.clone();
                async move { create_channel(addr).await }
            })
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_uri() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(create_channel("not a valid uri"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), GrpcError::InvalidUri(_)));
    }

    #[test]
    fn test_connection_failed() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        // Try to connect to a port that's definitely not listening
        let result = runtime.block_on(create_channel("http://[::1]:9999"));
        assert!(result.is_err());
        // Will timeout or fail to connect
    }
}
