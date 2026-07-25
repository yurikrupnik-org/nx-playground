use crate::{ConfigError, FromEnv, env_parse_or};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Server configuration for HTTP APIs.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub host: IpAddr,
    pub port: u16,
}

impl ServerConfig {
    /// Default bind host: all IPv4 interfaces (`0.0.0.0`).
    pub const DEFAULT_HOST: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    /// Default listen port.
    pub const DEFAULT_PORT: u16 = 8080;

    /// The socket address to bind to.
    ///
    /// Returns a `SocketAddr` rather than a formatted string so IPv6
    /// addresses are bracketed correctly (`[::1]:8080`) and it can be
    /// passed straight to `TcpListener::bind`.
    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

impl FromEnv for ServerConfig {
    /// Reads from environment variables, falling back to [`Default`]:
    /// - `HOST`: defaults to `0.0.0.0` (all interfaces)
    /// - `PORT`: defaults to `8080`
    ///
    /// A present-but-invalid `HOST` or `PORT` is a hard error.
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            host: env_parse_or("HOST", Self::DEFAULT_HOST)?,
            port: env_parse_or("PORT", Self::DEFAULT_PORT)?,
        })
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: Self::DEFAULT_HOST,
            port: Self::DEFAULT_PORT,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn test_server_config_from_env_with_defaults() {
        let _env = crate::test_env::guard();
        temp_env::with_vars([("HOST", None::<&str>), ("PORT", None::<&str>)], || {
            let config = ServerConfig::from_env().unwrap();
            assert_eq!(config.host, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
            assert_eq!(config.port, 8080);
            assert_eq!(config.addr().to_string(), "0.0.0.0:8080");
        });
    }

    #[test]
    fn test_server_config_from_env_with_custom_values() {
        let _env = crate::test_env::guard();
        temp_env::with_vars(
            [("HOST", Some("127.0.0.1")), ("PORT", Some("3000"))],
            || {
                let config = ServerConfig::from_env().unwrap();
                assert_eq!(config.host, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
                assert_eq!(config.port, 3000);
                assert_eq!(config.addr().to_string(), "127.0.0.1:3000");
            },
        );
    }

    #[test]
    fn test_server_config_from_env_partial_override() {
        let _env = crate::test_env::guard();
        temp_env::with_vars([("HOST", None::<&str>), ("PORT", Some("9000"))], || {
            let config = ServerConfig::from_env().unwrap();
            assert_eq!(config.host, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
            assert_eq!(config.port, 9000);
        });
    }

    #[test]
    fn test_server_config_from_env_invalid_port() {
        let _env = crate::test_env::guard();
        // Pin BOTH keys: `from_env` parses HOST before PORT, so an ambient HOST
        // (a dev `.env` often sets one) would fail first and mask the PORT error.
        temp_env::with_vars(
            [("HOST", None::<&str>), ("PORT", Some("not_a_number"))],
            || {
                let err = ServerConfig::from_env().unwrap_err();
                let msg = err.to_string();
                assert!(msg.contains("PORT"), "{msg}");
                assert!(msg.contains("not_a_number"), "{msg}");
            },
        );
    }

    #[test]
    fn test_server_config_from_env_port_out_of_range() {
        let _env = crate::test_env::guard();
        temp_env::with_vars([("HOST", None::<&str>), ("PORT", Some("99999"))], || {
            let err = ServerConfig::from_env().unwrap_err();
            assert!(err.to_string().contains("PORT"));
        });
    }

    #[test]
    fn test_server_config_from_env_port_zero() {
        // `0` is a valid `u16`; the OS picks an ephemeral port at bind time.
        let _env = crate::test_env::guard();
        temp_env::with_vars([("HOST", None::<&str>), ("PORT", Some("0"))], || {
            let config = ServerConfig::from_env().unwrap();
            assert_eq!(config.port, 0);
        });
    }

    #[test]
    fn test_server_config_from_env_invalid_host() {
        let _env = crate::test_env::guard();
        temp_env::with_vars([("HOST", Some("banana")), ("PORT", None::<&str>)], || {
            let err = ServerConfig::from_env().unwrap_err();
            assert!(err.to_string().contains("HOST"));
        });
    }

    #[test]
    fn test_server_config_addr_ipv6_is_bracketed() {
        let config = ServerConfig {
            host: IpAddr::V6(Ipv6Addr::LOCALHOST),
            port: 8080,
        };
        assert_eq!(config.addr().to_string(), "[::1]:8080");
    }

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.host, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(config.port, 8080);
    }
}
