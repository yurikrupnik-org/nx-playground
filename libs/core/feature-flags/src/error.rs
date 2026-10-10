use thiserror::Error;

/// Failures of the Flagsmith transport.
///
/// Only [`crate::FlagClient::new`] surfaces one of these to callers; a failed
/// *lookup* never does, because [`crate::FlagClient::flags`] degrades to
/// defaults instead of propagating.
#[derive(Debug, Error)]
pub enum FlagsError {
    #[error("failed to build the Flagsmith HTTP client: {source}")]
    ClientBuild {
        #[source]
        source: reqwest::Error,
    },

    #[error("Flagsmith request to {url} failed: {source}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("Flagsmith returned HTTP {status} for {url}")]
    Status { status: u16, url: String },

    #[error("failed to decode the Flagsmith response from {url}: {source}")]
    Decode {
        url: String,
        #[source]
        source: reqwest::Error,
    },
}
