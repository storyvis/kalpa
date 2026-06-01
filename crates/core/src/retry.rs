//! Retry with exponential backoff for transient failures.
//!
//! Production-grade retry logic for API calls that may fail due to
//! rate limiting, network issues, or transient server errors.

use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

use crate::error::{KalpaError, KalpaResult};

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of attempts (including the first).
    pub max_attempts: u32,
    /// Initial backoff duration.
    pub initial_backoff: Duration,
    /// Maximum backoff duration.
    pub max_backoff: Duration,
    /// Backoff multiplier (typically 2.0 for exponential).
    pub multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            multiplier: 2.0,
        }
    }
}

/// Determine if an error is retryable.
fn is_retryable(err: &KalpaError) -> bool {
    match err {
        KalpaError::Http(_) => true, // Network errors are always retryable
        KalpaError::RateLimited(_) => true,
        KalpaError::ProviderError { status, .. } => {
            // Retry on 429 (rate limit), 500, 502, 503, 504
            matches!(status, 429 | 500 | 502 | 503 | 504)
        }
        _ => false,
    }
}

/// Execute an async operation with exponential backoff retry.
///
/// Only retries on transient errors (network, rate limit, 5xx).
/// Non-retryable errors (auth, validation, 4xx) fail immediately.
///
/// # Example
/// ```ignore
/// use kalpa_core::retry::{retry_with_backoff, RetryConfig};
///
/// let result = retry_with_backoff(RetryConfig::default(), || async {
///     provider.complete(&request).await
/// }).await;
/// ```
pub async fn retry_with_backoff<F, Fut, T>(config: RetryConfig, operation: F) -> KalpaResult<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = KalpaResult<T>>,
{
    let mut backoff = config.initial_backoff;

    for attempt in 1..=config.max_attempts {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(err) => {
                if attempt == config.max_attempts || !is_retryable(&err) {
                    return Err(err);
                }

                warn!(
                    attempt = attempt,
                    max_attempts = config.max_attempts,
                    backoff_ms = backoff.as_millis() as u64,
                    error = %err,
                    "Retryable error, backing off"
                );

                sleep(backoff).await;
                backoff = Duration::from_secs_f64(
                    (backoff.as_secs_f64() * config.multiplier).min(config.max_backoff.as_secs_f64()),
                );
            }
        }
    }

    unreachable!("loop should have returned")
}
