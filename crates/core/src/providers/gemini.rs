//! Google Gemini (Generative Language API) provider — direct API-key access.
//!
//! Distinct from [`super::vertex`] (which uses OAuth + a GCP project). This hits
//! `generativelanguage.googleapis.com/.../{model}:generateContent` and returns
//! the model's interleaved `candidates[].content.parts[]` as our uniform
//! [`Part`] envelope — so a multimodal model like `gemini-3-pro-image`
//! ("Nano Banana Pro") yields text and image parts in one synchronous response.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{KalpaError, KalpaResult};
use crate::generation::{GenerationRequest, GenerationResponse, Modality, Part};
use crate::http::check_response;
use crate::provider::{GenerationProvider, JobHandle, PollStatus, SubmitOutcome};

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Direct Gemini API provider.
pub struct GeminiProvider {
    api_key: String,
    http: reqwest::Client,
}

// ── wire types (subset of the generateContent schema) ──

#[derive(Serialize)]
struct GenerateContentRequest {
    contents: Vec<GContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Serialize, Deserialize)]
struct GContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GPart>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inline_data: Option<InlineData>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: String,
    data: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    response_modalities: Vec<String>,
}

#[derive(Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(default)]
    usage_metadata: Option<UsageMetadata>,
}

#[derive(Deserialize)]
struct Candidate {
    content: Option<GContent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    #[serde(default)]
    prompt_token_count: u32,
    #[serde(default)]
    candidates_token_count: u32,
    #[serde(default)]
    total_token_count: u32,
}

impl GeminiProvider {
    /// Create a provider from a Gemini API key.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http: reqwest::Client::new(),
        }
    }

    fn modality_to_g(m: Modality) -> &'static str {
        match m {
            Modality::Image => "IMAGE",
            Modality::Audio => "AUDIO",
            _ => "TEXT",
        }
    }

    /// Map our request parts into Gemini `contents`.
    fn contents_from(request: &GenerationRequest) -> Vec<GContent> {
        let parts = request
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(GPart {
                    text: Some(text.clone()),
                    inline_data: None,
                }),
                Part::Image { b64_data: Some(data), mime, .. } => Some(GPart {
                    text: None,
                    inline_data: Some(InlineData {
                        mime_type: mime.clone().unwrap_or_else(|| "image/png".into()),
                        data: data.clone(),
                    }),
                }),
                _ => None,
            })
            .collect();
        vec![GContent {
            role: Some("user".into()),
            parts,
        }]
    }
}

#[async_trait]
impl GenerationProvider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn submit(&self, request: &GenerationRequest) -> KalpaResult<SubmitOutcome> {
        let model = request.model.split().1.to_string();
        let body = GenerateContentRequest {
            contents: Self::contents_from(request),
            generation_config: request.response_modalities.as_ref().map(|mods| GenerationConfig {
                response_modalities: mods.iter().map(|m| Self::modality_to_g(*m).to_string()).collect(),
            }),
        };

        let url = format!("{BASE_URL}/models/{model}:generateContent");
        let resp = self
            .http
            .post(&url)
            .query(&[("key", &self.api_key)])
            .json(&body)
            .send()
            .await?;
        let resp = check_response(resp, "gemini").await?;

        let parsed: GenerateContentResponse = resp.json().await?;
        let mut parts = Vec::new();
        if let Some(cand) = parsed.candidates.into_iter().next() {
            if let Some(content) = cand.content {
                for gp in content.parts {
                    if let Some(text) = gp.text {
                        parts.push(Part::Text { text });
                    } else if let Some(inline) = gp.inline_data {
                        parts.push(Part::Image {
                            url: None,
                            b64_data: Some(inline.data),
                            mime: Some(inline.mime_type),
                        });
                    }
                }
            }
        }

        let usage = parsed.usage_metadata.map(|u| crate::types::Usage {
            prompt_tokens: u.prompt_token_count,
            completion_tokens: u.candidates_token_count,
            total_tokens: u.total_token_count,
        });

        Ok(SubmitOutcome::Sync(GenerationResponse {
            model,
            parts,
            usage,
        }))
    }

    async fn poll(&self, _handle: &JobHandle) -> KalpaResult<PollStatus> {
        Err(KalpaError::Other("gemini is synchronous; poll not supported".into()))
    }
}
