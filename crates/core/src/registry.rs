//! Model registry: logical models and their per-provider bindings.
//!
//! A *logical model* (`flux-dev`) is the stable, user-facing capability with a
//! provider-neutral param contract. A *binding* is one concrete provider offering
//! of it (`fal` + `fal-ai/flux/dev`). The same logical model can have several
//! bindings; the registry resolves a [`ModelRef`] to one binding — pinned
//! (`provider:slug`) or selected by priority (logical slug).
//!
//! For now the catalog is seeded statically via [`Registry::with_defaults`]; in
//! svstudio it is built from the `models` / `model_providers` tables.

use serde::Serialize;

use crate::error::{KalpaError, KalpaResult};
use crate::generation::{Modality, ModelRef};

/// A logical model: the capability and its provider-neutral contract.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    /// Logical slug, e.g. "flux-dev".
    pub slug: String,
    /// Human-readable name.
    pub display_name: String,
    /// svid generation tags (72-86) this model can serve.
    pub supported_gen_types: Vec<u8>,
    /// Modalities accepted as input.
    pub input_modalities: Vec<Modality>,
    /// Modalities this model can emit (may be several).
    pub output_modalities: Vec<Modality>,
    /// Provider offerings of this model.
    pub bindings: Vec<Binding>,
}

/// One concrete provider offering of a logical model.
#[derive(Debug, Clone, Serialize)]
pub struct Binding {
    /// Provider name, e.g. "fal".
    pub provider: String,
    /// Provider-specific slug, e.g. "fal-ai/flux/dev".
    pub provider_slug: String,
    /// Optional region (e.g. a Vertex location).
    pub region: Option<String>,
    /// Whether this binding is queue-based (no synchronous result).
    pub async_only: bool,
    /// Selection order when the model is unpinned (lower = preferred).
    pub priority: i32,
}

/// A resolved model reference: the logical model plus the chosen binding.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub model_slug: String,
    pub binding: Binding,
}

/// The model catalog.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    models: Vec<ModelInfo>,
}

impl Registry {
    /// Build a registry from a catalog of logical models.
    pub fn new(models: Vec<ModelInfo>) -> Self {
        Self { models }
    }

    /// A small static catalog used by the CLI and early milestones.
    pub fn with_defaults() -> Self {
        let fal = |slug: &str| Binding {
            provider: "fal".into(),
            provider_slug: slug.into(),
            region: None,
            async_only: true,
            priority: 100,
        };
        let gemini = |slug: &str| Binding {
            provider: "gemini".into(),
            provider_slug: slug.into(),
            region: None,
            async_only: false,
            priority: 100,
        };
        let vertex = |slug: &str| Binding {
            provider: "vertex".into(),
            provider_slug: slug.into(),
            region: Some("us-central1".into()),
            async_only: false,
            priority: 200,
        };

        Self::new(vec![
            ModelInfo {
                slug: "flux-dev".into(),
                display_name: "FLUX.1 [dev]".into(),
                supported_gen_types: vec![73], // t2i
                input_modalities: vec![Modality::Text],
                output_modalities: vec![Modality::Image],
                bindings: vec![fal("fal-ai/flux/dev")],
            },
            // Nano Banana Pro: multimodal in, interleaved text+image out.
            ModelInfo {
                slug: "gemini-3-pro-image".into(),
                display_name: "Gemini 3 Pro Image (Nano Banana Pro)".into(),
                supported_gen_types: vec![73, 72, 78], // t2i, t2t, i2i
                input_modalities: vec![Modality::Text, Modality::Image],
                output_modalities: vec![Modality::Image, Modality::Text],
                bindings: vec![gemini("gemini-3-pro-image-preview")],
            },
            // Gemini 3.1 Flash Image on Vertex: multimodal in, image+text out.
            // Routed through Vertex `generateContent` (non-`imagen` slug).
            ModelInfo {
                slug: "gemini-3.1-flash-image".into(),
                display_name: "Gemini 3.1 Flash Image (Nano Banana 2)".into(),
                supported_gen_types: vec![73, 72, 78], // t2i, t2t, i2i
                input_modalities: vec![Modality::Text, Modality::Image],
                output_modalities: vec![Modality::Image, Modality::Text],
                bindings: vec![vertex("gemini-3.1-flash-image")],
            },
            ModelInfo {
                slug: "gemini-2.5-flash".into(),
                display_name: "Gemini 2.5 Flash".into(),
                supported_gen_types: vec![72], // t2t
                input_modalities: vec![Modality::Text, Modality::Image],
                output_modalities: vec![Modality::Text],
                bindings: vec![gemini("gemini-2.5-flash")],
            },
            // Imagen on Vertex (b64 image output).
            ModelInfo {
                slug: "imagen-4".into(),
                display_name: "Imagen 4".into(),
                supported_gen_types: vec![73], // t2i
                input_modalities: vec![Modality::Text],
                output_modalities: vec![Modality::Image],
                bindings: vec![vertex("imagen-4.0-generate-001")],
            },
            // OpenAI DALL-E 3 (image).
            ModelInfo {
                slug: "dall-e-3".into(),
                display_name: "DALL·E 3".into(),
                supported_gen_types: vec![73], // t2i
                input_modalities: vec![Modality::Text],
                output_modalities: vec![Modality::Image],
                bindings: vec![Binding {
                    provider: "openai".into(),
                    provider_slug: "dall-e-3".into(),
                    region: None,
                    async_only: false,
                    priority: 100,
                }],
            },
            // OpenAI GPT-4.1 (text).
            ModelInfo {
                slug: "gpt-4.1".into(),
                display_name: "GPT-4.1".into(),
                supported_gen_types: vec![72], // t2t
                input_modalities: vec![Modality::Text, Modality::Image],
                output_modalities: vec![Modality::Text],
                bindings: vec![Binding {
                    provider: "openai".into(),
                    provider_slug: "gpt-4.1".into(),
                    region: None,
                    async_only: false,
                    priority: 100,
                }],
            },
            // Anthropic Claude Sonnet (text).
            ModelInfo {
                slug: "claude-sonnet".into(),
                display_name: "Claude Sonnet 4.6".into(),
                supported_gen_types: vec![72], // t2t
                input_modalities: vec![Modality::Text, Modality::Image],
                output_modalities: vec![Modality::Text],
                bindings: vec![Binding {
                    provider: "claude".into(),
                    provider_slug: "claude-sonnet-4-6".into(),
                    region: None,
                    async_only: false,
                    priority: 100,
                }],
            },
            // Fal MiniMax video (text→video / image→video).
            ModelInfo {
                slug: "minimax-video".into(),
                display_name: "MiniMax Video 01".into(),
                supported_gen_types: vec![74, 79], // t2v, i2v
                input_modalities: vec![Modality::Text, Modality::Image],
                output_modalities: vec![Modality::Video],
                bindings: vec![fal("fal-ai/minimax/video-01")],
            },
            // OpenAI TTS (text→speech).
            ModelInfo {
                slug: "tts-1".into(),
                display_name: "OpenAI TTS".into(),
                supported_gen_types: vec![75], // t2s
                input_modalities: vec![Modality::Text],
                output_modalities: vec![Modality::Audio],
                bindings: vec![Binding {
                    provider: "openai".into(),
                    provider_slug: "tts-1".into(),
                    region: None,
                    async_only: false,
                    priority: 100,
                }],
            },
            // OpenAI Whisper (speech→text).
            ModelInfo {
                slug: "whisper-1".into(),
                display_name: "OpenAI Whisper".into(),
                supported_gen_types: vec![84], // s2t
                input_modalities: vec![Modality::Audio],
                output_modalities: vec![Modality::Text],
                bindings: vec![Binding {
                    provider: "openai".into(),
                    provider_slug: "whisper-1".into(),
                    region: None,
                    async_only: false,
                    priority: 100,
                }],
            },
        ])
    }

    /// Resolve a [`ModelRef`] to a concrete binding.
    ///
    /// - Pinned `"provider:provider_slug"` → that exact binding.
    /// - Logical `"slug"` → lowest-`priority` binding of the matching model.
    pub fn resolve(&self, model: &ModelRef) -> KalpaResult<Resolved> {
        let (provider, slug) = model.split();
        match provider {
            Some(prov) => self
                .models
                .iter()
                .find_map(|m| {
                    m.bindings
                        .iter()
                        .find(|b| b.provider == prov && b.provider_slug == slug)
                        .map(|b| Resolved {
                            model_slug: m.slug.clone(),
                            binding: b.clone(),
                        })
                })
                .ok_or_else(|| {
                    KalpaError::Config(format!("No binding for pinned model '{}'", model.0))
                }),
            None => {
                let m = self
                    .models
                    .iter()
                    .find(|m| m.slug == slug)
                    .ok_or_else(|| KalpaError::Config(format!("Unknown model '{}'", slug)))?;
                let binding = m
                    .bindings
                    .iter()
                    .min_by_key(|b| b.priority)
                    .ok_or_else(|| {
                        KalpaError::Config(format!("Model '{}' has no provider bindings", slug))
                    })?;
                Ok(Resolved {
                    model_slug: m.slug.clone(),
                    binding: binding.clone(),
                })
            }
        }
    }

    /// List logical models that can emit the given output modality.
    pub fn list(&self, modality: Modality) -> Vec<&ModelInfo> {
        self.models
            .iter()
            .filter(|m| m.output_modalities.contains(&modality))
            .collect()
    }

    /// All logical models.
    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_logical_and_pinned() {
        let reg = Registry::with_defaults();

        let r = reg.resolve(&"flux-dev".into()).unwrap();
        assert_eq!(r.binding.provider, "fal");
        assert_eq!(r.binding.provider_slug, "fal-ai/flux/dev");

        let r = reg.resolve(&"fal:fal-ai/flux/dev".into()).unwrap();
        assert_eq!(r.model_slug, "flux-dev");

        assert!(reg.resolve(&"nope".into()).is_err());
        assert!(reg.resolve(&"fal:does/not/exist".into()).is_err());
    }

    #[test]
    fn list_by_modality() {
        let reg = Registry::with_defaults();
        // flux-dev, gemini-3-pro-image, gemini-3.1-flash-image, imagen-4, dall-e-3 emit images.
        assert_eq!(reg.list(Modality::Image).len(), 5);
        // minimax-video emits video.
        assert_eq!(reg.list(Modality::Video).len(), 1);
        // gemini-3-pro-image, gemini-3.1-flash-image, gemini-2.5-flash, gpt-4.1,
        // claude-sonnet, whisper-1 emit text.
        assert_eq!(reg.list(Modality::Text).len(), 6);
        // tts-1 emits audio.
        assert_eq!(reg.list(Modality::Audio).len(), 1);
    }
}
