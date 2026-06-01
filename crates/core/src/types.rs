//! Common types shared across providers.

use serde::{Deserialize, Serialize};

/// Represents a single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// The role of the message sender (e.g., "system", "user", "assistant").
    pub role: Role,
    /// The content of the message.
    pub content: String,
}

/// The role of a message participant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Parameters for a text completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// The model identifier (e.g., "gemini-pro", "claude-3-opus").
    pub model: String,
    /// The conversation messages.
    pub messages: Vec<Message>,
    /// Maximum tokens to generate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Temperature for sampling (0.0 - 2.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Top-p nucleus sampling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Stop sequences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
}

/// A completion response from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// The generated text content.
    pub content: String,
    /// The model that produced this response.
    pub model: String,
    /// Token usage information.
    pub usage: Option<Usage>,
    /// The finish reason (e.g., "stop", "length").
    pub finish_reason: Option<String>,
}

/// Token usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Number of tokens in the prompt.
    pub prompt_tokens: u32,
    /// Number of tokens in the completion.
    pub completion_tokens: u32,
    /// Total tokens used.
    pub total_tokens: u32,
}

/// Parameters for an embedding request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    /// The model to use for embedding.
    pub model: String,
    /// The input texts to embed.
    pub input: Vec<String>,
}

/// An embedding response from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    /// The generated embeddings (one per input).
    pub embeddings: Vec<Vec<f32>>,
    /// The model that produced the embeddings.
    pub model: String,
    /// Token usage information.
    pub usage: Option<Usage>,
}

/// Parameters for an image generation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationRequest {
    /// The model to use for generation.
    pub model: String,
    /// The prompt describing the image to generate.
    pub prompt: String,
    /// Number of images to generate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// Desired image size (e.g., "1024x1024").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
}

/// An image generation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationResponse {
    /// Generated images (as URLs or base64 data).
    pub images: Vec<GeneratedImage>,
    /// The model used.
    pub model: String,
}

/// A single generated image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedImage {
    /// URL to the generated image (if available).
    pub url: Option<String>,
    /// Base64-encoded image data (if available).
    pub b64_data: Option<String>,
}

/// Parameters for a video generation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoGenerationRequest {
    /// The model to use for generation.
    pub model: String,
    /// The prompt describing the video to generate.
    pub prompt: String,
    /// Optional image URL for image-to-video models.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    /// Duration in seconds (if supported by model).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<i32>,
}

/// A video generation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoGenerationResponse {
    /// Generated videos.
    pub videos: Vec<GeneratedVideo>,
    /// The model used.
    pub model: String,
}

/// A single generated video.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedVideo {
    /// URL to the generated video.
    pub url: String,
}

// Fal-specific types are defined in `crate::providers::falai` and re-exported
// here for backward compatibility with CLI code that imports from `kalpa_core::types`.
pub use crate::providers::falai::{FalLogEntry, FalQueueStatus, FalQueueSubmitResponse};
