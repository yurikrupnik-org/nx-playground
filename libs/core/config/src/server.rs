use crate::{ConfigError, FromEnv, env_or_default};
use std::net::Ipv4Addr;

/// Server configuration for HTTP APIs
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl ServerConfig {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    /// Get the server address as "host:port"
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl FromEnv for ServerConfig {
    /// Reads from environment variables with sensible defaults:
    /// - HOST: defaults to Ipv4Addr::UNSPECIFIED (0.0.0.0 - all interfaces)
    /// - PORT: defaults to 8080
    fn from_env() -> Result<Self, ConfigError> {
        let host = env_or_default("HOST", &Ipv4Addr::UNSPECIFIED.to_string());
        let port = env_or_default("PORT", "8080")
            .parse()
            .map_err(|e| ConfigError::ParseError {
                key: "PORT".to_string(),
                details: format!("{}", e),
            })?;

        Ok(Self { host, port })
    }
}

impl Default for ServerConfig {
    async fn default() -> Result<Self, ConfigError> {
        Ok(Self {
            host: Ipv4Addr::UNSPECIFIED.to_string(),
            port: env_or_default("PORT", "8080")
              .parse()
              .map_err(|e| ConfigError::ParseError {
                key: "PORT".to_string(),
                details: format!("{}", e),
              })?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_from_env_with_defaults() {
        temp_env::with_vars([("HOST", None::<&str>), ("PORT", None::<&str>)], || {
            let config = ServerConfig::from_env().unwrap();
            assert_eq!(config.host, "0.0.0.0");
            assert_eq!(config.port, 8080);
            assert_eq!(config.address(), "0.0.0.0:8080");
        });
    }

    #[test]
    fn test_server_config_from_env_with_custom_values() {
        temp_env::with_vars(
            [("HOST", Some("127.0.0.1")), ("PORT", Some("3000"))],
            || {
                let config = ServerConfig::from_env().unwrap();
                assert_eq!(config.host, "127.0.0.1");
                assert_eq!(config.port, 3000);
                assert_eq!(config.address(), "127.0.0.1:3000");
            },
        );
    }

    #[test]
    fn test_server_config_from_env_partial_override() {
        temp_env::with_vars([("HOST", None::<&str>), ("PORT", Some("9000"))], || {
            let config = ServerConfig::from_env().unwrap();
            assert_eq!(config.host, "0.0.0.0");
            assert_eq!(config.port, 9000);
        });
    }

    #[test]
    fn test_server_config_from_env_invalid_port() {
        temp_env::with_var("PORT", Some("not_a_number"), || {
            let result = ServerConfig::from_env();
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("PORT"));
        });
    }

    #[test]
    fn test_server_config_from_env_port_out_of_range() {
        temp_env::with_var("PORT", Some("99999"), || {
            let result = ServerConfig::from_env();
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("PORT"));
        });
    }

    #[test]
    fn test_server_config_new() {
        let config = ServerConfig::new("192.168.1.1".to_string(), 5000);
        assert_eq!(config.host, "192.168.1.1");
        assert_eq!(config.port, 5000);
    }

    #[test]
    fn test_server_config_address() {
        let config = ServerConfig::new("localhost".to_string(), 8080);
        assert_eq!(config.address(), "localhost:8080");
    }

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.host, Ipv4Addr::UNSPECIFIED.to_string());
        assert_eq!(config.port, 8080);
    }
}
