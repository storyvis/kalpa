//! Unified error types for kalpa.

use thiserror::Error;

/// The main error type used across all kalpa crates.
#[derive(Debug, Error)]
pub enum KalpaError {
    /// HTTP/network-level error from the underlying client.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

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
    RateLimited(String),

    /// Invalid configuration or parameters.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Any other error.
    #[error("{0}")]
    Other(String),
}

/// A convenience Result type for kalpa operations.
pub type KalpaResult<T> = Result<T, KalpaError>;
