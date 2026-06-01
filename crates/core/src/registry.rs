//! Model registry — single source of truth for supported models per provider/content type.
//!
//! Eliminates model list duplication across provider implementations and CLI commands.

use crate::config::Provider;

/// Content type for generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Text,
    Image,
    Video,
}

/// Model entry with metadata.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub id: &'static str,
    pub is_default: bool,
}

impl ModelEntry {
    const fn new(id: &'static str) -> Self {
        Self { id, is_default: false }
    }

    const fn default_model(id: &'static str) -> Self {
        Self { id, is_default: true }
    }
}

/// Get all supported models for a provider/content-type combination.
/// This is the single source of truth — providers and CLI both use this.
pub fn supported_models(provider: Provider, kind: ContentKind) -> &'static [ModelEntry] {
    match (provider, kind) {
        // --- Gemini ---
        (Provider::Gemini, ContentKind::Text) => &[
            ModelEntry::default_model("gemini-2.5-flash"),
            ModelEntry::new("gemini-2.0-flash"),
            ModelEntry::new("gemini-2.0-flash-exp"),
            ModelEntry::new("gemini-1.5-pro"),
            ModelEntry::new("gemini-1.5-flash"),
        ],
        (Provider::Gemini, ContentKind::Image) => &[
            ModelEntry::default_model("gemini-2.5-flash"),
        ],
        (Provider::Gemini, ContentKind::Video) => &[
            ModelEntry::default_model("gemini-2.5-flash"),
        ],

        // --- Vertex AI ---
        (Provider::Vertex, ContentKind::Text) => &[
            ModelEntry::new("gemini-3.1-flash"),
            ModelEntry::new("gemini-3-pro"),
            ModelEntry::default_model("gemini-2.5-flash"),
            ModelEntry::new("gemini-2.0-flash"),
            ModelEntry::new("gemini-2.0-flash-exp"),
            ModelEntry::new("gemini-1.5-pro"),
            ModelEntry::new("gemini-1.5-flash"),
        ],
        (Provider::Vertex, ContentKind::Image) => &[
            ModelEntry::default_model("imagen-4.0-generate-001"),
            ModelEntry::new("imagen-3.0-generate-001"),
            ModelEntry::new("imagen-3.0-generate-002"),
            ModelEntry::new("imagen-3.0-fast-generate-001"),
        ],
        (Provider::Vertex, ContentKind::Video) => &[
            ModelEntry::new("veo-3.0-generate"),
            ModelEntry::new("veo-3.0-fast-generate-preview"),
            ModelEntry::default_model("veo-2.0-generate-001"),
        ],

        // --- OpenAI ---
        (Provider::OpenAI, ContentKind::Text) => &[
            ModelEntry::default_model("gpt-4.1"),
            ModelEntry::new("gpt-4.1-mini"),
            ModelEntry::new("gpt-4.1-preview"),
            ModelEntry::new("gpt-4"),
            ModelEntry::new("gpt-4-turbo"),
            ModelEntry::new("gpt-4-turbo-preview"),
            ModelEntry::new("gpt-3.5-turbo"),
            ModelEntry::new("gpt-3.5-turbo-16k"),
        ],
        (Provider::OpenAI, ContentKind::Image) => &[
            ModelEntry::default_model("dall-e-3"),
            ModelEntry::new("dall-e-2"),
            ModelEntry::new("gpt-image-1.5"),
        ],
        (Provider::OpenAI, ContentKind::Video) => &[],

        // --- Claude ---
        (Provider::Claude, ContentKind::Text) => &[
            ModelEntry::new("claude-opus-4-7"),
            ModelEntry::new("claude-opus-4-6"),
            ModelEntry::default_model("claude-sonnet-4-6"),
            ModelEntry::new("claude-haiku-4-5-20251001"),
            ModelEntry::new("claude-3-opus"),
            ModelEntry::new("claude-3-sonnet"),
            ModelEntry::new("claude-3-haiku"),
        ],
        (Provider::Claude, ContentKind::Image) => &[],
        (Provider::Claude, ContentKind::Video) => &[],

        // --- Fal.ai ---
        (Provider::Fal, ContentKind::Text) => &[],
        (Provider::Fal, ContentKind::Image) => &[
            ModelEntry::new("fal-ai/flux/dev"),
            ModelEntry::new("fal-ai/flux/schnell"),
            ModelEntry::new("fal-ai/flux-pro"),
            ModelEntry::new("fal-ai/flux-realism"),
            ModelEntry::new("fal-ai/recraft-v3"),
            ModelEntry::new("fal-ai/aura-flow"),
            ModelEntry::new("fal-ai/stable-diffusion-v3-medium"),
            ModelEntry::default_model("fal-ai/fast-sdxl"),
        ],
        (Provider::Fal, ContentKind::Video) => &[
            // Text-to-Video
            ModelEntry::new("fal-ai/minimax/video-01"),
            ModelEntry::new("fal-ai/minimax/video-01-live"),
            ModelEntry::new("fal-ai/hunyuan-video"),
            ModelEntry::new("fal-ai/mochi-v1"),
            ModelEntry::default_model("fal-ai/kling-video/v1/standard/text-to-video"),
            ModelEntry::new("fal-ai/kling-video/v1.5/standard/text-to-video"),
            ModelEntry::new("fal-ai/kling-video/v1.6/standard/text-to-video"),
            ModelEntry::new("fal-ai/kling-video/v2.1/master/text-to-video"),
            ModelEntry::new("fal-ai/kling-video/v2.6/pro/text-to-video"),
            ModelEntry::new("fal-ai/wan/v2.2-a14b/text-to-video"),
            ModelEntry::new("fal-ai/ltx-2/text-to-video"),
            ModelEntry::new("fal-ai/ltx-2.3/text-to-video"),
            ModelEntry::new("fal-ai/veo3"),
            ModelEntry::new("fal-ai/veo3.1"),
            ModelEntry::new("bytedance/seedance-2.0/text-to-video"),
            ModelEntry::new("bytedance/seedance-2.0/fast/text-to-video"),
            // Image-to-Video
            ModelEntry::new("fal-ai/veo2/image-to-video"),
            ModelEntry::new("fal-ai/veo3/image-to-video"),
            ModelEntry::new("fal-ai/luma-dream-machine/image-to-video"),
            ModelEntry::new("fal-ai/kling-video/v2.1/master/image-to-video"),
            ModelEntry::new("fal-ai/kling-video/v1.6/pro/image-to-video"),
            ModelEntry::new("fal-ai/minimax/video-01-live/image-to-video"),
            ModelEntry::new("fal-ai/pixverse/v4.5/image-to-video"),
            ModelEntry::new("bytedance/seedance-2.0/image-to-video"),
            // Legacy
            ModelEntry::new("fal-ai/kling-video/v1/standard/image-to-video"),
            ModelEntry::new("fal-ai/kling-video/v1.5/standard/image-to-video"),
            ModelEntry::new("fal-ai/minimax/video-01/image-to-video"),
            ModelEntry::new("fal-ai/wan/v2.2-a14b/image-to-video"),
            ModelEntry::new("fal-ai/luma-dream-machine"),
        ],
    }
}

/// Get model IDs as a simple string slice (for trait implementations).
pub fn model_ids(provider: Provider, kind: ContentKind) -> Vec<&'static str> {
    supported_models(provider, kind).iter().map(|m| m.id).collect()
}

/// Get the default model for a provider/content-type.
pub fn default_model(provider: Provider, kind: ContentKind) -> Option<&'static str> {
    supported_models(provider, kind)
        .iter()
        .find(|m| m.is_default)
        .map(|m| m.id)
}

/// Check if a model is supported for the given provider/content-type.
pub fn is_model_supported(provider: Provider, kind: ContentKind, model: &str) -> bool {
    supported_models(provider, kind).iter().any(|m| m.id == model)
}

/// Get providers that support a given content kind.
pub fn providers_for_kind(kind: ContentKind) -> Vec<(Provider, &'static str)> {
    Provider::all()
        .iter()
        .filter(|p| !supported_models(**p, kind).is_empty())
        .map(|p| (*p, p.as_str()))
        .collect()
}
