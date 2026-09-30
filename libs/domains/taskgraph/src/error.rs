use thiserror::Error;

#[derive(Debug, Error)]
pub enum TaskgraphError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{path}: invalid YAML: {source}")]
    Yaml {
        path: String,
        #[source]
        source: serde_yaml_ng::Error,
    },

    #[error("NATS: {0}")]
    Nats(String),
}

pub type TaskgraphResult<T> = Result<T, TaskgraphError>;
