//! Unified error types for kalpa.

use std::time::Duration;
use thiserror::Error;

/// The main error type used across all kalpa crates.
#[derive(Debug, Error)]
pub enum KalpaError {
    /// HTTP/network-level error from the underlying client.
    #[error("HTTP error: {0}")]
    Http(reqwest::Error),

    /// Serialization/deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The provider returned an error response.
    #[error("Provider error ({status}): {message}")]
    ProviderError { status: u16, message: String },

    /// Authentication error (missing or invalid credentials).
    #[error("Authentication error: {0}")]
    Auth(String),

    /// Rate limiting or quota exceeded.
    #[error("Rate limited: {0}")]
    RateLimited(RateLimit),

    /// Invalid configuration or parameters.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Any other error.
    #[error("{0}")]
    Other(String),
}

/// A rate-limit refusal: what the provider said, and how long it asked to be
/// left alone (`Retry-After`, or Google's `RetryInfo.retryDelay`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimit {
    pub message: String,
    pub retry_after: Option<Duration>,
}

impl std::fmt::Display for RateLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for RateLimit {
    fn from(message: String) -> Self {
        Self {
            message,
            retry_after: None,
        }
    }
}

impl From<&str> for RateLimit {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

impl KalpaError {
    /// How long the provider asked to be left alone, when it said.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited(limit) => limit.retry_after,
            _ => None,
        }
    }
}

/// A convenience Result type for kalpa operations.
pub type KalpaResult<T> = Result<T, KalpaError>;

impl From<reqwest::Error> for KalpaError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error.without_url())
    }
}
