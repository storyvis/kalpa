//! OpenAI provider implementation.

use async_trait::async_trait;
use kalpa_libgen::openai;
use crate::error::{KalpaError, KalpaResult};
use crate::generation::{GenerationRequest, GenerationResponse, Part};
use crate::http::check_response;
use crate::provider::{
    CompletionProvider, GenerationProvider, ImageGenerationProvider, JobHandle, PollStatus,
    SubmitOutcome,
};
use crate::types::{
    CompletionRequest, CompletionResponse, ImageGenerationRequest, ImageGenerationResponse,
    GeneratedImage, Message, Role, Usage,
};

/// OpenAI provider for GPT models, DALL-E, and Sora.
pub struct OpenAIProvider {
    client: openai::Client,
    /// Authenticated raw client for endpoints the generated client lacks
    /// (audio speech / transcriptions).
    http: reqwest::Client,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider.
    ///
    /// # Arguments
    /// * `api_key` - OpenAI API key
    pub fn new(api_key: String) -> Self {
        let http = reqwest::Client::builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key))
                        .unwrap(),
                );
                headers
            })
            .build()
            .unwrap();

        let client = openai::Client::new_with_client("https://api.openai.com", http.clone());

        Self { client, http }
    }

    /// Convert kalpa Message to OpenAI Message
    fn convert_message(message: &Message) -> openai::types::Message {
        let role = match message.role {
            Role::User => openai::types::MessageRole::User,
            Role::Assistant => openai::types::MessageRole::Assistant,
            Role::System => openai::types::MessageRole::System,
        };

        openai::types::Message {
            role: Some(role),
            content: Some(message.content.clone()),
        }
    }
}

#[async_trait]
impl CompletionProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn supported_models(&self) -> &[&str] {
        &[
            // GPT-4.1 series
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4.1-preview",
            // GPT-4 series
            "gpt-4",
            "gpt-4-turbo",
            "gpt-4-turbo-preview",
            // GPT-3.5 series
            "gpt-3.5-turbo",
            "gpt-3.5-turbo-16k",
        ]
    }

    async fn complete(&self, request: &CompletionRequest) -> KalpaResult<CompletionResponse> {
        // Convert messages
        let messages: Vec<_> = request.messages.iter().map(Self::convert_message).collect();

        // Build OpenAI request
        let openai_request = openai::types::ChatCompletionRequest {
            model: request.model.clone(),
            messages,
            temperature: request.temperature.map(|t| t as f64),
            max_tokens: request.max_tokens.map(|t| t as i64),
        };

        // Make the API call
        let response = self
            .client
            .chat_completions(&openai_request)
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("OpenAI API error: {}", e),
            })?;

        // Extract the response
        let choice = response
            .choices
            .first()
            .ok_or_else(|| KalpaError::ProviderError {
                status: 500,
                message: "No choices in response".to_string(),
            })?;

        let content = choice
            .message
            .as_ref()
            .and_then(|m| m.content.clone())
            .ok_or_else(|| KalpaError::ProviderError {
                status: 500,
                message: "No content in message".to_string(),
            })?;

        let usage = response.usage.as_ref().map(|u| Usage {
            prompt_tokens: u.input_tokens.unwrap_or(0) as u32,
            completion_tokens: u.output_tokens.unwrap_or(0) as u32,
            total_tokens: u.total_tokens.unwrap_or(0) as u32,
        });

        Ok(CompletionResponse {
            content,
            model: request.model.clone(),
            usage,
            finish_reason: None,
        })
    }

    async fn health_check(&self) -> KalpaResult<()> {
        // Simple health check
        let request = CompletionRequest {
            model: "gpt-4.1-mini".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "Hi".to_string(),
            }],
            max_tokens: Some(5),
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        self.complete(&request).await?;
        Ok(())
    }
}

#[async_trait]
impl ImageGenerationProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn supported_models(&self) -> &[&str] {
        &[
            "dall-e-3",
            "dall-e-2",
            "gpt-image-1.5",  // If this is a real model
        ]
    }

    async fn generate_image(
        &self,
        request: &ImageGenerationRequest,
    ) -> KalpaResult<ImageGenerationResponse> {
        // Build OpenAI request
        let openai_request = openai::types::ImageGenerationRequest {
            model: request.model.clone(),
            prompt: request.prompt.clone(),
            n: Some(1),
            size: request.size.clone(),
        };

        // Make the API call
        let response = self
            .client
            .create_image(&openai_request)
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("OpenAI image generation error: {}", e),
            })?;

        // Extract images
        let images: Vec<GeneratedImage> = response
            .data
            .iter()
            .map(|item| GeneratedImage {
                url: item.url.clone(),
                b64_data: item.b64_json.clone(),
            })
            .collect();

        Ok(ImageGenerationResponse {
            images,
            model: request.model.clone(),
        })
    }
}

// ─── Unified GenerationProvider ──────────────────────────────────────────────
//
// dall-e / gpt-image models → image (sync, url or b64 parts); other models →
// chat (sync, text part).


#[async_trait]
impl GenerationProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn submit(&self, request: &GenerationRequest) -> KalpaResult<SubmitOutcome> {
        let model = request.model.split().1.to_string();
        let prompt = request.text_prompt()?;

        if model.contains("dall-e") || model.contains("gpt-image") {
            let img_req = ImageGenerationRequest {
                model: model.clone(),
                prompt,
                n: Some(1),
                size: request
                    .params
                    .as_ref()
                    .and_then(|p| p.get("size"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
            };
            let resp = self.generate_image(&img_req).await?;
            let parts = resp
                .images
                .into_iter()
                .map(|img| Part::Image {
                    url: img.url,
                    b64_data: img.b64_data,
                    mime: Some("image/png".into()),
                })
                .collect();
            Ok(SubmitOutcome::Sync(GenerationResponse { model, parts, usage: None }))
        } else {
            let comp_req = CompletionRequest {
                model: model.clone(),
                messages: vec![Message { role: Role::User, content: prompt }],
                max_tokens: None,
                temperature: None,
                top_p: None,
                stop_sequences: None,
            };
            let resp = self.complete(&comp_req).await?;
            Ok(SubmitOutcome::Sync(GenerationResponse {
                model,
                parts: vec![Part::Text { text: resp.content }],
                usage: resp.usage,
            }))
        }
    }

    async fn poll(&self, _handle: &JobHandle) -> KalpaResult<PollStatus> {
        Err(KalpaError::Other("openai generation is synchronous; poll not supported".into()))
    }
}

// ─── Audio: TTS (speech) + STT (transcription) ───────────────────────────────
//
// The generated client lacks audio endpoints, so these call the REST API
// directly with the authenticated `http` client.

use crate::generation::{SpeechRequest, TranscriptionRequest};
use crate::provider::{SpeechProvider, TranscriptionProvider};

#[async_trait]
impl SpeechProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn synthesize(&self, request: &SpeechRequest) -> KalpaResult<GenerationResponse> {
        let model = request.model.split().1.to_string();
        let format = request.format.clone().unwrap_or_else(|| "mp3".into());
        let body = serde_json::json!({
            "model": model,
            "input": request.input,
            "voice": request.voice.clone().unwrap_or_else(|| "alloy".into()),
            "response_format": format,
        });

        let resp = self
            .http
            .post("https://api.openai.com/v1/audio/speech")
            .json(&body)
            .send()
            .await?;
        let resp = check_response(resp, "openai").await?;

        use base64::Engine;
        let bytes = resp.bytes().await?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let mime = match format.as_str() {
            "wav" => "audio/wav",
            "opus" => "audio/opus",
            "aac" => "audio/aac",
            "flac" => "audio/flac",
            _ => "audio/mpeg",
        };

        Ok(GenerationResponse {
            model,
            parts: vec![Part::Audio {
                url: None,
                b64_data: Some(b64),
                mime: Some(mime.into()),
            }],
            usage: None,
        })
    }
}

#[async_trait]
impl TranscriptionProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn transcribe(&self, request: &TranscriptionRequest) -> KalpaResult<GenerationResponse> {
        let model = request.model.split().1.to_string();

        // Fetch the audio bytes (uploaded/stored separately, referenced by URL).
        let audio = reqwest::get(&request.audio_url).await?.bytes().await?;

        let mut form = reqwest::multipart::Form::new()
            .text("model", model.clone())
            .part(
                "file",
                reqwest::multipart::Part::bytes(audio.to_vec()).file_name("audio"),
            );
        if let Some(lang) = &request.language {
            form = form.text("language", lang.clone());
        }

        let resp = self
            .http
            .post("https://api.openai.com/v1/audio/transcriptions")
            .multipart(form)
            .send()
            .await?;
        let resp = check_response(resp, "openai").await?;

        #[derive(serde::Deserialize)]
        struct TranscriptionResponse {
            text: String,
        }
        let parsed: TranscriptionResponse = resp.json().await?;
        Ok(GenerationResponse {
            model,
            parts: vec![Part::Text { text: parsed.text }],
            usage: None,
        })
    }
}
