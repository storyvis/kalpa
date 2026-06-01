//! Unified multimodal generation types.
//!
//! A single model may take mixed input (text + images) and emit *interleaved*
//! text and media in one response (e.g. `gemini-3-pro-image`). So generation is
//! modelled as a request carrying a model reference + content parts, and a
//! response carrying a uniform `parts` array — never a flat `images`/`content`.
//!
//! The older per-modality structs in [`crate::types`] remain for the existing
//! providers; `From` conversions bridge them to this envelope.

use serde::{Deserialize, Serialize};

use crate::types::Usage;

/// A modality of generation input or output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
    Embedding,
}

/// One piece of multimodal content. Requests and responses are both `Vec<Part>`,
/// which is what lets a model return text and an image in the same response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Part {
    /// Plain text.
    Text { text: String },
    /// An image, by URL or inline base64 data.
    Image {
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        b64_data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
    /// A video, by URL.
    Video {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
    /// Audio, by URL or inline base64 data.
    Audio {
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        b64_data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
    /// An embedding vector.
    Embedding { vector: Vec<f32> },
}

impl Part {
    /// Convenience constructor for a text part.
    pub fn text(s: impl Into<String>) -> Self {
        Part::Text { text: s.into() }
    }

    /// Convenience constructor for an image URL part.
    pub fn image_url(url: impl Into<String>) -> Self {
        Part::Image {
            url: Some(url.into()),
            b64_data: None,
            mime: None,
        }
    }
}

/// A reference to a model in a request: either a logical slug (the server selects
/// a provider binding) or a pinned `provider:slug`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRef(pub String);

impl ModelRef {
    /// Returns `(Some(provider), slug)` if pinned (`"fal:fal-ai/flux/dev"`),
    /// otherwise `(None, slug)` for a logical reference.
    pub fn split(&self) -> (Option<&str>, &str) {
        match self.0.split_once(':') {
            // Only treat the prefix as a provider when the suffix is non-empty;
            // protects against slugs that legitimately contain a colon later.
            Some((prov, rest)) if !rest.is_empty() => (Some(prov), rest),
            _ => (None, self.0.as_str()),
        }
    }
}

impl<T: Into<String>> From<T> for ModelRef {
    fn from(s: T) -> Self {
        ModelRef(s.into())
    }
}

/// A unified multimodal generation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationRequest {
    /// The model to run (logical slug or pinned `provider:slug`).
    pub model: ModelRef,
    /// Input content parts (prompt text, source images, audio, ...).
    pub parts: Vec<Part>,
    /// Provider-neutral, model-specific parameters (size, duration, voice, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    /// Optional hint constraining which output modalities the model should emit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_modalities: Option<Vec<Modality>>,
}

impl GenerationRequest {
    /// Build a simple text-prompt request.
    pub fn prompt(model: impl Into<ModelRef>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            parts: vec![Part::text(prompt)],
            params: None,
            response_modalities: None,
        }
    }

    /// Extract the first text part as a prompt string.
    ///
    /// Returns an error if no text part is present — use this in provider
    /// `submit()` implementations to avoid repeating the same extraction logic.
    pub fn text_prompt(&self) -> crate::error::KalpaResult<String> {
        self.parts
            .iter()
            .find_map(|p| match p {
                Part::Text { text } => Some(text.clone()),
                _ => None,
            })
            .ok_or_else(|| crate::error::KalpaError::ProviderError {
                status: 400,
                message: "request requires at least one text part".into(),
            })
    }
}

/// A unified multimodal generation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationResponse {
    /// The model (or binding slug) that produced this response.
    pub model: String,
    /// Output content parts — may interleave text and media.
    pub parts: Vec<Part>,
    /// Token / unit usage, when reported by the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl GenerationResponse {
    /// All image URLs in the response, in order.
    pub fn image_urls(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Image { url: Some(u), .. } => Some(u.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Concatenated text of all text parts.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// Parameters for a text-to-speech (TTS) request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechRequest {
    /// The model to use (e.g. `tts-1`).
    pub model: ModelRef,
    /// The text to synthesize.
    pub input: String,
    /// Optional voice identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    /// Optional output format (e.g. `mp3`, `wav`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

/// Parameters for a speech-to-text (STT) transcription request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionRequest {
    /// The model to use (e.g. `whisper-1`).
    pub model: ModelRef,
    /// URL of the audio to transcribe (uploaded/stored separately).
    pub audio_url: String,
    /// Optional source language hint (ISO code).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_round_trip_interleaved() {
        let resp = GenerationResponse {
            model: "gemini-3-pro-image".into(),
            parts: vec![
                Part::text("here is the logo"),
                Part::image_url("https://example/img.png"),
                Part::text("and a note"),
            ],
            usage: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: GenerationResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.image_urls(), vec!["https://example/img.png"]);
        assert_eq!(back.text(), "here is the logoand a note");
    }

    #[test]
    fn model_ref_split() {
        assert_eq!(ModelRef::from("flux-dev").split(), (None, "flux-dev"));
        assert_eq!(
            ModelRef::from("fal:fal-ai/flux/dev").split(),
            (Some("fal"), "fal-ai/flux/dev")
        );
    }
}
