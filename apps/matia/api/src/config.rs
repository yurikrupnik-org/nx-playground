use core_config::{AppInfo, FromEnv, app_info, server::ServerConfig};

pub use core_config::Environment;

#[derive(Clone, Debug)]
pub struct Config {
    pub app: AppInfo,
    pub server: ServerConfig,
    pub environment: Environment,
    pub frontend_url: String,
}

impl Config {
    pub fn from_env() -> eyre::Result<Self> {
        let environment = Environment::from_env();
        let server = ServerConfig::from_env()?;
        let frontend_url = core_config::env_or_default("FRONTEND_URL", "http://localhost:3001");

        Ok(Self {
            app: app_info!(),
            server,
            environment,
            frontend_url,
        })
    }
}
