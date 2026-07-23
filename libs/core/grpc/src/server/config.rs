//! Server configuration loaded from environment variables.

use crate::error::GrpcError;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

/// Configuration for gRPC server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// IP to bind to (default: `::1`, IPv6 localhost)
    pub host: IpAddr,
    /// Port to listen on (default: 50051)
    pub port: u16,
    /// Enable Zstd compression (default: true)
    pub enable_compression: bool,
    /// Maximum message size for decoding (default: 8MB)
    pub max_decoding_message_size: usize,
    /// Maximum message size for encoding (default: 8MB)
    pub max_encoding_message_size: usize,
    /// TCP keepalive interval in seconds (default: 60)
    pub keepalive_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: IpAddr::V6(Ipv6Addr::LOCALHOST),
            port: 50051,
            enable_compression: true,
            max_decoding_message_size: 8 * 1024 * 1024, // 8MB
            max_encoding_message_size: 8 * 1024 * 1024, // 8MB
            keepalive_secs: 60,
        }
    }
}

impl ServerConfig {
    /// Create a new server config with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load configuration from environment variables.
    ///
    /// Reads:
    /// - `GRPC_HOST` (default: `::1`; `[::1]`-style brackets are accepted)
    /// - `GRPC_PORT` (default: 50051)
    /// - `GRPC_COMPRESSION` (default: true; `false`/`0` disable)
    /// - `GRPC_MAX_MESSAGE_SIZE` (default: 8388608 / 8MB)
    ///
    /// A present-but-unparseable value is a hard error so misconfiguration
    /// fails at startup instead of silently falling back to defaults.
    pub fn from_env() -> Result<Self, GrpcError> {
        let mut config = Self::default();

        if let Ok(host) = std::env::var("GRPC_HOST") {
            // Deployment manifests commonly spell IPv6 hosts as "[::1]".
            let trimmed = host
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .unwrap_or(&host);
            config.host = trimmed.parse().map_err(|e| {
                GrpcError::InvalidConfig(format!("GRPC_HOST: invalid IP {host:?}: {e}"))
            })?;
        }

        if let Ok(port) = std::env::var("GRPC_PORT") {
            config.port = port.parse().map_err(|e| {
                GrpcError::InvalidConfig(format!("GRPC_PORT: invalid port {port:?}: {e}"))
            })?;
        }

        if let Ok(compression) = std::env::var("GRPC_COMPRESSION") {
            config.enable_compression = compression != "false" && compression != "0";
        }

        if let Ok(size) = std::env::var("GRPC_MAX_MESSAGE_SIZE") {
            let size: usize = size.parse().map_err(|e| {
                GrpcError::InvalidConfig(format!("GRPC_MAX_MESSAGE_SIZE: invalid size {size:?}: {e}"))
            })?;
            config.max_decoding_message_size = size;
            config.max_encoding_message_size = size;
        }

        Ok(config)
    }

    /// Set the host to bind to.
    pub fn with_host(mut self, host: IpAddr) -> Self {
        self.host = host;
        self
    }

    /// Set the port to listen on.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Enable or disable compression.
    pub fn with_compression(mut self, enable: bool) -> Self {
        self.enable_compression = enable;
        self
    }

    /// Set maximum message size.
    pub fn with_max_message_size(mut self, size: usize) -> Self {
        self.max_decoding_message_size = size;
        self.max_encoding_message_size = size;
        self
    }

    /// Get the socket address to bind to.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_default_config() {
        let config = ServerConfig::default();
        assert_eq!(config.host, IpAddr::V6(Ipv6Addr::LOCALHOST));
        assert_eq!(config.port, 50051);
        assert!(config.enable_compression);
        assert_eq!(config.socket_addr().to_string(), "[::1]:50051");
    }

    #[test]
    fn test_builder_pattern() {
        let config = ServerConfig::new()
            .with_host(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
            .with_port(8080)
            .with_compression(false);

        assert_eq!(config.host, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(config.port, 8080);
        assert!(!config.enable_compression);
    }

    #[test]
    fn test_from_env_accepts_bracketed_ipv6() {
        temp_env::with_var("GRPC_HOST", Some("[::1]"), || {
            let config = ServerConfig::from_env().unwrap();
            assert_eq!(config.host, IpAddr::V6(Ipv6Addr::LOCALHOST));
        });
    }

    #[test]
    fn test_from_env_invalid_port_fails_fast() {
        temp_env::with_var("GRPC_PORT", Some("not-a-port"), || {
            let err = ServerConfig::from_env().unwrap_err();
            assert!(err.to_string().contains("GRPC_PORT"), "{err}");
        });
    }

    #[test]
    fn test_from_env_invalid_max_message_size_fails_fast() {
        temp_env::with_var("GRPC_MAX_MESSAGE_SIZE", Some("8MB"), || {
            let err = ServerConfig::from_env().unwrap_err();
            assert!(err.to_string().contains("GRPC_MAX_MESSAGE_SIZE"), "{err}");
        });
    }
}
