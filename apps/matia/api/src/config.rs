use core_config::{AppInfo, FromEnv, app_info, server::ServerConfig, ConfigError};

pub use core_config::Environment;

#[derive(Clone, Debug)]
pub struct Config {
    pub app: AppInfo,
    pub server: ServerConfig,
    pub environment: Environment,
    pub frontend_url: String,
}

impl Config {
  // fn odsa(key: &str) -> Option<String> {
  //   if let Ok(s) = std::env::var(key) {
  //     return Some(s);
  //   }
  //   None
  // }
  pub fn from_env() -> Result<Self, ConfigError> {
        let environment = Environment::from_env();
        let server = ServerConfig::from_env()?;
        let frontend_url = core_config::env_or_default("FRONTEND_URL", "http://localhost:3001");
        // let odsa_url = Self::odsa("asd");
        // // match odsa_url {
        // //   Some(url) => {
        // //     println!("odsa url: {}", url);
        // //   },
        // //   None => {
        // //     println!("odsa url not found");
        // //   }
        // }
        Ok(Self {
            app: app_info!(),
            server,
            environment,
            frontend_url,
        })
    }
}
