use crate::github::GitHubError;

#[derive(Debug, thiserror::Error)]
pub enum InsightsError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    GitHub(#[from] GitHubError),
    #[error("nats: {0}")]
    Nats(String),
    #[error("otlp export: {0}")]
    Otlp(String),
}

pub type InsightsResult<T> = Result<T, InsightsError>;
