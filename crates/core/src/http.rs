//! HTTP response utilities shared across providers.
//!
//! Provides a single `check_response()` function that maps HTTP status codes
//! to the appropriate `KalpaError` variant, eliminating the repetitive
//! "check 429, check !success, extract body" pattern in every provider.

use crate::error::{KalpaError, KalpaResult};

/// Check an HTTP response status, converting failures to `KalpaError`.
///
/// - 429 → `KalpaError::RateLimited`
/// - Any non-2xx → `KalpaError::ProviderError` with the status code and body
/// - 2xx → returns the response unchanged for further processing
///
/// `provider_name` is used only for the fallback message on 429 when the body
/// is empty.
pub async fn check_response(
    resp: reqwest::Response,
    provider_name: &str,
) -> KalpaResult<reqwest::Response> {
    let status = resp.status();
    if status.as_u16() == 429 {
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| format!("{} 429", provider_name));
        return Err(KalpaError::RateLimited(body));
    }
    if !status.is_success() {
        let code = status.as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(KalpaError::ProviderError {
            status: code,
            message: body,
        });
    }
    Ok(resp)
}
