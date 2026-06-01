//! Provider traits that all AI backends must implement.
//!
//! These traits define the contract for interacting with AI services.
//! Each provider (Gemini, Vertex AI, Fal, etc.) implements the relevant
//! traits to provide a unified interface.

use async_trait::async_trait;

use crate::error::KalpaResult;
use crate::types::{
    CompletionRequest, CompletionResponse, EmbeddingRequest, EmbeddingResponse,
    ImageGenerationRequest, ImageGenerationResponse, VideoGenerationRequest,
    VideoGenerationResponse,
};

/// Trait for providers that support text completion / chat.
#[async_trait]
pub trait CompletionProvider: Send + Sync {
    /// Returns the provider name (e.g., "gemini", "vertex", "openai").
    fn name(&self) -> &str;

    /// Returns the list of models supported by this provider.
    fn supported_models(&self) -> &[&str];

    /// Perform a text completion / chat request.
    async fn complete(&self, request: &CompletionRequest) -> KalpaResult<CompletionResponse>;

    /// Check if the provider is properly configured and reachable.
    async fn health_check(&self) -> KalpaResult<()>;
}

/// Trait for providers that support text embeddings.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Returns the provider name.
    fn name(&self) -> &str;

    /// Returns the list of embedding models supported.
    fn supported_models(&self) -> &[&str];

    /// Generate embeddings for the given input texts.
    async fn embed(&self, request: &EmbeddingRequest) -> KalpaResult<EmbeddingResponse>;
}

/// Trait for providers that support image generation.
#[async_trait]
pub trait ImageGenerationProvider: Send + Sync {
    /// Returns the provider name.
    fn name(&self) -> &str;

    /// Returns the list of image generation models supported.
    fn supported_models(&self) -> &[&str];

    /// Generate images from a text prompt.
    async fn generate_image(
        &self,
        request: &ImageGenerationRequest,
    ) -> KalpaResult<ImageGenerationResponse>;
}

/// Trait for providers that support video generation.
#[async_trait]
pub trait VideoGenerationProvider: Send + Sync {
    /// Returns the provider name.
    fn name(&self) -> &str;

    /// Returns the list of video generation models supported.
    fn supported_models(&self) -> &[&str];

    /// Generate videos from a text prompt or image.
    async fn generate_video(
        &self,
        request: &VideoGenerationRequest,
    ) -> KalpaResult<VideoGenerationResponse>;
}
