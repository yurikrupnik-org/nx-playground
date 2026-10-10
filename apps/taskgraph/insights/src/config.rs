//! Single load point for every environment-derived setting (`AppConfig: FromEnv`).

use std::str::FromStr;
use std::time::Duration;

use core_config::server::ServerConfig;
use core_config::{ConfigError, Environment, FromEnv, env_or_default, env_parse_or, env_required};

pub const DEFAULT_REPOSITORY: &str = "yurikrupnik-org/nx-playground";
pub const DEFAULT_CI_WORKFLOWS: &str = ".github/workflows/ci-optimized.yml";

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub environment: Environment,
    pub server: ServerConfig,
    pub database_url: String,
    pub github_token: String,
    /// `owner/name`.
    pub github_repository: String,
    pub nats_url: String,
    /// OTLP/gRPC endpoint for historical CI traces; unset = no trace export.
    pub otlp_endpoint: Option<String>,
    pub sync_interval: Duration,
    pub backfill_days: i64,
    /// Workflow paths counted by the scorecard.
    pub ci_workflows: Vec<String>,
}

impl FromEnv for AppConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            environment: Environment::from_env()?,
            server: ServerConfig::from_env()?,
            database_url: env_required("DATABASE_URL")?,
            github_token: env_required("GITHUB_TOKEN")?,
            github_repository: env_or_default("GITHUB_REPOSITORY", DEFAULT_REPOSITORY),
            nats_url: env_or_default("NATS_URL", "nats://localhost:4222"),
            otlp_endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .ok()
                .filter(|s| !s.is_empty()),
            sync_interval: env_parse_or(
                "INSIGHTS_SYNC_INTERVAL",
                Interval(Duration::from_secs(600)),
            )?
            .0,
            backfill_days: env_parse_or("INSIGHTS_BACKFILL_DAYS", 90)?,
            ci_workflows: env_or_default("INSIGHTS_CI_WORKFLOWS", DEFAULT_CI_WORKFLOWS)
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
        })
    }
}

/// `90s`, `10m`, `1h`, or bare seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval(pub Duration);

impl FromStr for Interval {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let (number, unit) = s.split_at(split);
        let n: u64 = number
            .parse()
            .map_err(|_| format!("expected <number>[s|m|h], got {s:?}"))?;
        let secs = match unit {
            "" | "s" => n,
            "m" => n * 60,
            "h" => n * 3600,
            other => return Err(format!("unknown unit {other:?} (use s, m or h)")),
        };
        if secs == 0 {
            return Err("interval must be positive".into());
        }
        Ok(Self(Duration::from_secs(secs)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_accepts_units_and_rejects_nonsense() {
        let secs = |s: &str| s.parse::<Interval>().map(|i| i.0.as_secs());
        assert_eq!(secs("10m"), Ok(600));
        assert_eq!(secs("90s"), Ok(90));
        assert_eq!(secs("1h"), Ok(3600));
        assert_eq!(secs("45"), Ok(45));
        assert!(secs("0m").is_err());
        assert!(secs("10 minutes").is_err());
        assert!(secs("m").is_err());
    }
}
