//! Provider traits that all AI backends must implement.
//!
//! These traits define the contract for interacting with AI services.
//! Each provider (Gemini, Vertex AI, Fal, etc.) implements the relevant
//! traits to provide a unified interface.

use async_trait::async_trait;

use crate::error::KalpaResult;
use crate::generation::{GenerationRequest, GenerationResponse, SpeechRequest, TranscriptionRequest};
use crate::types::{
    CompletionRequest, CompletionResponse, EmbeddingRequest, EmbeddingResponse,
    ImageGenerationRequest, ImageGenerationResponse, VideoGenerationRequest,
    VideoGenerationResponse,
};

/// Opaque handle to an in-flight async/queue-based generation, used to poll.
#[derive(Debug, Clone)]
pub struct JobHandle {
    /// Provider-side request/operation id (for logging + provenance).
    pub provider_request_id: String,
    /// URL to poll for status, when the provider uses one.
    pub poll_url: Option<String>,
    /// URL to fetch the final result, when distinct from `poll_url`.
    pub response_url: Option<String>,
}

/// Result of submitting a generation. A synchronous provider finishes inline;
/// a queue-based provider returns a handle to poll. The `RateLimited` decorator
/// holds the limiter permit across the whole lifecycle in both cases.
pub enum SubmitOutcome {
    /// Work completed synchronously (HTTP response carried the result).
    Sync(GenerationResponse),
    /// Work enqueued; poll `handle` to terminal state.
    Async(JobHandle),
}

/// Status returned while polling an async generation.
pub enum PollStatus {
    /// Waiting in the provider queue.
    InQueue { position: Option<u32> },
    /// Actively running.
    InProgress,
    /// Done — carries the result.
    Completed(GenerationResponse),
    /// Failed — carries the provider error message.
    Failed(String),
}

/// Unified multimodal generation provider. Every modality endpoint is a thin
/// wrapper over this; `submit`/`poll` cover both the synchronous and the
/// queue-based provider lifecycles (see the design doc's AIMD section).
#[async_trait]
pub trait GenerationProvider: Send + Sync {
    /// Provider name (e.g. "fal", "openai").
    fn name(&self) -> &str;

    /// Submit a generation. Returns inline result or a handle to poll.
    /// On rate limiting, return `KalpaError::RateLimited` so the limiter backs off.
    async fn submit(&self, request: &GenerationRequest) -> KalpaResult<SubmitOutcome>;

    /// Poll an async generation for its current status.
    async fn poll(&self, handle: &JobHandle) -> KalpaResult<PollStatus>;
}

/// Provider that supports text-to-speech.
#[async_trait]
pub trait SpeechProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn synthesize(&self, request: &SpeechRequest) -> KalpaResult<GenerationResponse>;
}

/// Provider that supports speech-to-text transcription.
#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn transcribe(&self, request: &TranscriptionRequest) -> KalpaResult<GenerationResponse>;
}

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
