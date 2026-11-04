//! # kalpa-libgen
//!
//! Client SDK generation library for AI provider APIs.
//!
//! This crate uses [progenitor](https://docs.rs/progenitor) to generate
//! type-safe Rust client libraries from OpenAPI 3.0 specification files.
//!
//! Official source snapshots, provenance, and deterministic compatibility
//! projections live in `specs/` (see its README). Build-time Rust output lives
//! in Cargo's `OUT_DIR`; regeneration does not require network access.
//!
//! To add a provider, add its projected `specs/<provider>.json` and an included
//! module below. `build.rs` discovers root JSON specifications automatically.

// Generated SDK modules - automatically included from build.rs output

pub mod gemini {
    include!(concat!(env!("OUT_DIR"), "/gemini.rs"));
}

pub mod vertex {
    include!(concat!(env!("OUT_DIR"), "/vertex.rs"));
}

pub mod openai {
    include!(concat!(env!("OUT_DIR"), "/openai.rs"));
}

pub mod falai {
    include!(concat!(env!("OUT_DIR"), "/falai.rs"));
}

pub mod claude {
    include!(concat!(env!("OUT_DIR"), "/claude.rs"));
}

/// Re-export for convenience in downstream crates.
pub use progenitor_client;

/// Maximum JSON/media response accepted by generated provider clients (64 MiB).
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum BodyError {
    #[error("HTTP response read failed")]
    Http(reqwest::Error),
    #[error("Provider response exceeds {0} bytes")]
    TooLarge(usize),
}

/// Read a response with a streaming byte cap, even without Content-Length.
pub async fn read_bounded(
    mut response: reqwest::Response,
    cap: usize,
) -> Result<Vec<u8>, BodyError> {
    if response
        .content_length()
        .is_some_and(|len| len > cap as u64)
    {
        return Err(BodyError::TooLarge(cap));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(BodyError::Http)? {
        if chunk.len() > cap.saturating_sub(bytes.len()) {
            return Err(BodyError::TooLarge(cap));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

// Injected into every generated operation before Progenitor's deserializer.
// Progenitor hooks borrow the response and cannot enforce a chunked body cap.
#[doc(hidden)]
pub async fn bounded_response<E>(
    response: reqwest::Response,
) -> Result<reqwest::Response, progenitor_client::Error<E>> {
    let status = response.status();
    let version = response.version();
    let headers = response.headers().clone();
    // Error payloads can echo credentials; leave the status but discard the body.
    let bytes = if status.is_success() {
        read_bounded(response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|error| match error {
                BodyError::Http(error) => {
                    progenitor_client::Error::CommunicationError(error.without_url())
                }
                BodyError::TooLarge(_) => progenitor_client::Error::Custom(error.to_string()),
            })?
    } else {
        Vec::new()
    };
    let mut bounded = http::Response::new(bytes);
    *bounded.status_mut() = status;
    *bounded.version_mut() = version;
    *bounded.headers_mut() = headers;
    // The new body has a known, bounded length and is no longer chunked.
    bounded
        .headers_mut()
        .remove(http::header::TRANSFER_ENCODING);
    bounded.headers_mut().remove(http::header::CONTENT_LENGTH);
    let bounded = reqwest::Response::from(bounded);
    if !status.is_success() {
        return Err(progenitor_client::Error::UnexpectedResponse(bounded));
    }
    Ok(bounded)
}

// Model IDs are multiple path segments; encode each, preserving only separators.
#[doc(hidden)]
pub fn encode_model_path<E>(model: &str) -> Result<String, progenitor_client::Error<E>> {
    if model
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(progenitor_client::Error::InvalidRequest(
            "Invalid model path".into(),
        ));
    }
    Ok(model
        .split('/')
        .map(progenitor_client::encode_path)
        .collect::<Vec<_>>()
        .join("/"))
}
