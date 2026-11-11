//! OpenAI provider implementation.

use crate::error::{KalpaError, KalpaResult};
use crate::generation::{GenerationRequest, GenerationResponse, Part};
use crate::http::check_response;
use crate::provider::{
    CompletionProvider, GenerationProvider, ImageGenerationProvider, JobHandle, PollStatus,
    SubmitOutcome,
};
use crate::types::{
    CompletionRequest, CompletionResponse, GeneratedImage, ImageGenerationRequest,
    ImageGenerationResponse, Message, Role, Usage,
};
use async_trait::async_trait;
use kalpa_libgen::openai;
use serde_json::Value;

/// The public API base, `/v1` included (the OpenAI SDKs' convention).
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Image fields passed through from `GenerationRequest::params`. Not
/// `response_format`: the gpt-image models reject it, and either answer
/// (url or b64) is handled.
const IMAGE_PARAMS: &[&str] = &[
    "n",
    "size",
    "quality",
    "style",
    "background",
    "output_format",
    "output_compression",
    "moderation",
];

/// OpenAI provider for GPT models, DALL-E / gpt-image, and Sora.
pub struct OpenAIProvider {
    client: openai::Client,
    /// Authenticated raw client for endpoints the generated client lacks or
    /// whose errors it cannot carry (images, videos, audio).
    http: reqwest::Client,
    base_url: String,
}

impl OpenAIProvider {
    /// Use another OpenAI-compatible API base (a proxy or gateway), `/v1`
    /// included, e.g. `https://proxy.example.com/v1`. Empty keeps the default.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let base = base_url.into().trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return self;
        }
        // The generated client's paths start with `/v1`.
        let origin = base.strip_suffix("/v1").unwrap_or(&base);
        self.client = openai::Client::new_with_client(origin, self.http.clone());
        self.base_url = base;
        self
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base_url)
    }

    /// Create a new OpenAI provider.
    ///
    /// # Arguments
    /// * `api_key` - OpenAI API key
    pub fn new(api_key: String) -> Self {
        let http = crate::http::client_builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    crate::http::secret_header(&format!("Bearer {}", api_key)),
                );
                headers
            })
            .build()
            .unwrap();

        let client = openai::Client::new_with_client("https://api.openai.com", http.clone());

        Self {
            client,
            http,
            base_url: DEFAULT_BASE_URL.into(),
        }
    }

    /// `POST /images/generations`, as parts. `params` may hold any of
    /// [`IMAGE_PARAMS`]; others are not sent.
    /// With `inputs` (image bytes and their MIME types) it is an edit:
    /// `POST /images/edits` as a form, the images as `image[]` files.
    async fn create_images(
        &self,
        model: &str,
        prompt: &str,
        params: Option<&Value>,
        inputs: Vec<(Vec<u8>, String)>,
    ) -> KalpaResult<Vec<Part>> {
        let body = image_body(model, prompt, params);
        let resp = if inputs.is_empty() {
            self.http
                .post(self.url("images/generations"))
                .json(&body)
                .send()
                .await?
        } else {
            let mut form = reqwest::multipart::Form::new();
            for (k, v) in body.as_object().into_iter().flatten() {
                let text = v
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| v.to_string());
                form = form.text(k.clone(), text);
            }
            for (i, (bytes, mime)) in inputs.into_iter().enumerate() {
                let ext = mime.rsplit('/').next().unwrap_or("png").to_string();
                let part = reqwest::multipart::Part::bytes(bytes)
                    .file_name(format!("image-{i}.{ext}"))
                    .mime_str(&mime)
                    .map_err(|_| KalpaError::ProviderError {
                        status: 400,
                        message: format!("openai: unsupported image type {mime}"),
                    })?;
                form = form.part("image[]", part);
            }
            self.http
                .post(self.url("images/edits"))
                .multipart(form)
                .send()
                .await?
        };
        let parsed: ImagesResponse =
            crate::http::read_json(check_response(resp, "openai").await?).await?;
        let mime = image_mime(params);
        let parts: Vec<Part> = parsed
            .data
            .into_iter()
            .filter(|d| d.url.is_some() || d.b64_json.is_some())
            .map(|d| Part::Image {
                url: d.url,
                b64_data: d.b64_json,
                mime: Some(mime.into()),
            })
            .collect();
        if parts.is_empty() {
            return Err(KalpaError::ProviderError {
                status: 502,
                message: "openai returned no image".into(),
            });
        }
        Ok(parts)
    }

    /// `POST /videos`: a Sora job, polled by [`GenerationProvider::poll`].
    async fn create_video(
        &self,
        model: &str,
        prompt: &str,
        params: Option<&Value>,
    ) -> KalpaResult<JobHandle> {
        let resp = self
            .http
            .post(self.url("videos"))
            .json(&video_body(model, prompt, params))
            .send()
            .await?;
        let job: VideoJob = crate::http::read_json(check_response(resp, "openai").await?).await?;
        Ok(JobHandle {
            provider_request_id: job.id,
            poll_url: None,
            response_url: None,
        })
    }

    /// The finished clip. Its URL needs the key, so the bytes are fetched
    /// here and passed on as a `data:` URL.
    async fn video_content(&self, id: &str) -> KalpaResult<Part> {
        use base64::Engine;
        let resp = self
            .http
            .get(self.url(&format!("videos/{}/content", path_segment(id)?)))
            .send()
            .await?;
        let resp = check_response(resp, "openai").await?;
        let bytes = crate::http::read_bounded(resp, crate::http::MAX_RESPONSE_BYTES).await?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(Part::Video {
            url: format!("data:video/mp4;base64,{b64}"),
            mime: Some("video/mp4".into()),
        })
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
            max_tokens: None,
            max_completion_tokens: request.max_tokens.map(|t| t as i64),
        };

        // Make the API call
        let response = self
            .client
            .chat_completions(&openai_request)
            .await
            .map_err(crate::http::generated_error)?;

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
            prompt_tokens: u.prompt_tokens.unwrap_or(0) as u32,
            completion_tokens: u.completion_tokens.unwrap_or(0) as u32,
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
            "gpt-image-1",
            "gpt-image-1.5",
            "gpt-image-1-mini",
        ]
    }

    async fn generate_image(
        &self,
        request: &ImageGenerationRequest,
    ) -> KalpaResult<ImageGenerationResponse> {
        let params = serde_json::json!({ "n": request.n.unwrap_or(1), "size": request.size });
        let images: Vec<GeneratedImage> = self
            .create_images(&request.model, &request.prompt, Some(&params), Vec::new())
            .await?
            .into_iter()
            .filter_map(|part| match part {
                Part::Image { url, b64_data, .. } => Some(GeneratedImage { url, b64_data }),
                _ => None,
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
// dall-e / gpt-image models → image (sync, url or b64 parts); sora models →
// video (async job: submit, poll, content); other models → chat (sync, text).

#[async_trait]
impl GenerationProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn submit(&self, request: &GenerationRequest) -> KalpaResult<SubmitOutcome> {
        let model = request.model.split().1.to_string();
        let prompt = request.text_prompt()?;
        let kind = media_kind(&model);
        let has_image = request
            .parts
            .iter()
            .any(|p| matches!(p, Part::Image { .. }));
        if kind == MediaKind::Video && has_image {
            // Refused rather than dropped: image-to-video is not wired for Sora.
            return Err(KalpaError::ProviderError {
                status: 400,
                message: format!("openai: an input image is not supported for {model}"),
            });
        }

        if kind == MediaKind::Image {
            let parts = self
                .create_images(
                    &model,
                    &prompt,
                    request.params.as_ref(),
                    edit_inputs(request)?,
                )
                .await?;
            Ok(SubmitOutcome::Sync(GenerationResponse {
                model,
                parts,
                usage: None,
            }))
        } else if kind == MediaKind::Video {
            Ok(SubmitOutcome::Async(
                self.create_video(&model, &prompt, request.params.as_ref())
                    .await?,
            ))
        } else {
            let comp_req = CompletionRequest {
                model: model.clone(),
                messages: vec![Message {
                    role: Role::User,
                    content: prompt,
                }],
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

    /// Only Sora jobs are asynchronous.
    async fn poll(&self, handle: &JobHandle) -> KalpaResult<PollStatus> {
        let id = path_segment(&handle.provider_request_id)?;
        let resp = self
            .http
            .get(self.url(&format!("videos/{id}")))
            .send()
            .await?;
        let job: VideoJob = crate::http::read_json(check_response(resp, "openai").await?).await?;
        Ok(match job.status.as_str() {
            "completed" => PollStatus::Completed(GenerationResponse {
                model: job.model.unwrap_or_default(),
                parts: vec![self.video_content(&job.id).await?],
                usage: None,
            }),
            "failed" => {
                let reason = job
                    .error
                    .and_then(|e| serde_json::to_vec(&serde_json::json!({ "error": e })).ok())
                    .and_then(|body| crate::http::error_message(&body))
                    .unwrap_or_else(|| "the video failed".into());
                PollStatus::Failed(format!("openai: {reason}"))
            }
            "queued" => PollStatus::InQueue { position: None },
            _ => PollStatus::InProgress,
        })
    }

    async fn cancel(&self, handle: &JobHandle) -> KalpaResult<()> {
        let id = path_segment(&handle.provider_request_id)?;
        let resp = self
            .http
            .delete(self.url(&format!("videos/{id}")))
            .send()
            .await?;
        check_response(resp, "openai").await.map(|_| ())
    }
}

/// The images an edit sends, as bytes. OpenAI's edits take uploads only, so
/// an image by URL is refused: the caller fetches it (it holds the SSRF policy).
fn edit_inputs(request: &GenerationRequest) -> KalpaResult<Vec<(Vec<u8>, String)>> {
    use base64::Engine;
    let refuse = |message: &str| KalpaError::ProviderError {
        status: 400,
        message: format!("openai: {message}"),
    };
    request
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Image { b64_data, mime, .. } => Some((b64_data, mime)),
            _ => None,
        })
        .map(|(b64, mime)| {
            let b64 = b64
                .as_deref()
                .ok_or_else(|| refuse("an image to edit must be sent as bytes, not a URL"))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|_| refuse("an image to edit is not valid base64"))?;
            Ok((bytes, mime.clone().unwrap_or_else(|| "image/png".into())))
        })
        .collect()
}

#[derive(Debug, PartialEq)]
enum MediaKind {
    Image,
    Video,
    Text,
}

fn media_kind(model: &str) -> MediaKind {
    if model.starts_with("sora") {
        MediaKind::Video
    } else if model.contains("dall-e") || model.contains("gpt-image") {
        MediaKind::Image
    } else {
        MediaKind::Text
    }
}

fn image_body(model: &str, prompt: &str, params: Option<&Value>) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("model".into(), model.into());
    body.insert("prompt".into(), prompt.into());
    if let Some(p) = params.and_then(Value::as_object) {
        for k in IMAGE_PARAMS {
            if let Some(v) = p.get(*k).filter(|v| !v.is_null()) {
                body.insert((*k).into(), v.clone());
            }
        }
    }
    Value::Object(body)
}

/// gpt-image models answer in `output_format` (png by default).
fn image_mime(params: Option<&Value>) -> &'static str {
    match params
        .and_then(|p| p.get("output_format"))
        .and_then(Value::as_str)
    {
        Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

/// `seconds` is text to OpenAI ("4", "8", "12"); callers may send a number,
/// or `duration` as fal and Vertex take it.
fn video_body(model: &str, prompt: &str, params: Option<&Value>) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("model".into(), model.into());
    body.insert("prompt".into(), prompt.into());
    let p = params.and_then(Value::as_object);
    match p.and_then(|p| p.get("seconds").or_else(|| p.get("duration"))) {
        Some(Value::String(s)) if !s.is_empty() => {
            body.insert("seconds".into(), s.clone().into());
        }
        Some(Value::Number(n)) => {
            body.insert("seconds".into(), n.to_string().into());
        }
        _ => {}
    }
    if let Some(size) = p.and_then(|p| p.get("size")).and_then(Value::as_str) {
        body.insert("size".into(), size.into());
    }
    Value::Object(body)
}

/// A provider job id placed in a URL path: only `[A-Za-z0-9_-]`.
fn path_segment(id: &str) -> KalpaResult<&str> {
    if !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Ok(id)
    } else {
        Err(KalpaError::ProviderError {
            status: 502,
            message: "openai returned an invalid job id".into(),
        })
    }
}

#[derive(serde::Deserialize)]
struct ImagesResponse {
    #[serde(default)]
    data: Vec<ImageData>,
}

#[derive(serde::Deserialize)]
struct ImageData {
    url: Option<String>,
    b64_json: Option<String>,
}

#[derive(serde::Deserialize)]
struct VideoJob {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    error: Option<Value>,
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
            .post(self.url("audio/speech"))
            .json(&body)
            .send()
            .await?;
        let resp = check_response(resp, "openai").await?;

        use base64::Engine;
        let bytes = crate::http::read_bounded(resp, crate::http::MAX_RESPONSE_BYTES).await?;
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
        let audio = crate::http::download_public_audio(&request.audio_url).await?;

        let mut form = reqwest::multipart::Form::new()
            .text("model", model.clone())
            .part(
                "file",
                reqwest::multipart::Part::bytes(audio).file_name("audio"),
            );
        if let Some(lang) = &request.language {
            form = form.text("language", lang.clone());
        }

        let resp = self
            .http
            .post(self.url("audio/transcriptions"))
            .multipart(form)
            .send()
            .await?;
        let resp = check_response(resp, "openai").await?;

        #[derive(serde::Deserialize)]
        struct TranscriptionResponse {
            text: String,
        }
        let parsed: TranscriptionResponse = crate::http::read_json(resp).await?;
        Ok(GenerationResponse {
            model,
            parts: vec![Part::Text { text: parsed.text }],
            usage: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::ModelRef;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn request(model: &str, params: Value) -> GenerationRequest {
        GenerationRequest {
            model: ModelRef(model.into()),
            parts: vec![Part::text("waves")],
            params: Some(params),
            response_modalities: None,
        }
    }

    fn provider(server: &MockServer) -> OpenAIProvider {
        OpenAIProvider::new("test-key".into()).with_base_url(format!("{}/v1/", server.uri()))
    }

    #[test]
    fn models_are_sorted_by_kind() {
        assert_eq!(media_kind("gpt-image-1.5"), MediaKind::Image);
        assert_eq!(media_kind("dall-e-3"), MediaKind::Image);
        assert_eq!(media_kind("sora-2-pro"), MediaKind::Video);
        assert_eq!(media_kind("gpt-4.1"), MediaKind::Text);
    }

    #[test]
    fn image_body_passes_only_openai_fields() {
        let params = json!({"size": "1024x1024", "quality": "low", "routing": {"target": "x"}, "response_format": "url", "n": 1});
        assert_eq!(
            image_body("gpt-image-1", "a cat", Some(&params)),
            json!({"model": "gpt-image-1", "prompt": "a cat", "size": "1024x1024", "quality": "low", "n": 1})
        );
    }

    #[test]
    fn video_seconds_are_sent_as_text() {
        let b = video_body(
            "sora-2",
            "waves",
            Some(&json!({"seconds": 4, "size": "720x1280"})),
        );
        assert_eq!(
            b,
            json!({"model": "sora-2", "prompt": "waves", "seconds": "4", "size": "720x1280"})
        );
        assert_eq!(
            video_body("sora-2", "w", Some(&json!({"duration": "8"})))["seconds"],
            "8"
        );
    }

    #[test]
    fn job_ids_cannot_leave_their_path_segment() {
        assert!(path_segment("video_68d7512d").is_ok());
        for bad in ["", "../files", "a/b", "a?b"] {
            assert!(path_segment(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn images_use_the_base_url_and_answer_as_parts() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(
                json!({"model": "gpt-image-1.5", "quality": "low"}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "UE5H"}]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let p = provider(&server);
        let SubmitOutcome::Sync(done) = p
            .submit(&request(
                "gpt-image-1.5",
                json!({"quality": "low", "output_format": "webp"}),
            ))
            .await
            .unwrap()
        else {
            panic!("async")
        };
        let [Part::Image { b64_data, mime, .. }] = done.parts.as_slice() else {
            panic!("{:?}", done.parts)
        };
        assert_eq!(b64_data.as_deref(), Some("UE5H"));
        assert_eq!(mime.as_deref(), Some("image/webp"));
    }

    #[tokio::test]
    async fn an_image_error_carries_openais_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"error": {"message": "The model 'dall-e-3' does not exist.", "code": "invalid_value"}}),
            ))
            .mount(&server)
            .await;
        let Err(KalpaError::ProviderError { status, message }) = provider(&server)
            .submit(&request("dall-e-3", json!({})))
            .await
        else {
            panic!("expected a provider error")
        };
        assert_eq!(status, 400);
        assert_eq!(
            message,
            "openai request failed: The model 'dall-e-3' does not exist. (invalid_value)"
        );
    }

    #[tokio::test]
    async fn an_image_by_url_or_for_sora_is_refused_not_dropped() {
        let server = MockServer::start().await;
        for model in ["gpt-image-1.5", "sora-2"] {
            let mut req = request(model, json!({}));
            req.parts.push(Part::image_url("https://example.com/a.png"));
            assert!(
                matches!(
                    provider(&server).submit(&req).await,
                    Err(KalpaError::ProviderError { status: 400, .. })
                ),
                "{model}"
            );
        }
    }

    /// An input image makes it an edit: a form to `/images/edits` with the
    /// bytes as `image[]` and the prompt beside them.
    #[tokio::test]
    async fn an_image_with_bytes_is_edited() {
        use wiremock::matchers::body_string_contains;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_string_contains(
                "name=\"image[]\"; filename=\"image-0.png\"",
            ))
            .and(body_string_contains("PNGDATA"))
            .and(body_string_contains("make it night"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": [{"b64_json": "RURJVA=="}]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut req = request("gpt-image-1.5", json!({"size": "1024x1024"}));
        req.parts = vec![
            Part::text("make it night"),
            Part::Image {
                url: None,
                b64_data: Some("UE5HREFUQQ==".into()),
                mime: Some("image/png".into()),
            },
        ];
        let SubmitOutcome::Sync(done) = provider(&server).submit(&req).await.unwrap() else {
            panic!("async")
        };
        assert!(matches!(
            done.parts.as_slice(),
            [Part::Image { b64_data: Some(b), .. }] if b == "RURJVA=="
        ));
    }

    /// Submit, poll while queued, poll when done, the clip fetched with the
    /// key into a `data:` URL, then cancel.
    #[tokio::test]
    async fn a_sora_job_is_submitted_polled_and_downloaded() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(
                json!({"model": "sora-2", "seconds": "4"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id": "video_1", "status": "queued"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/video_1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id": "video_1", "status": "queued"})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/video_1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"id": "video_1", "status": "completed", "model": "sora-2"}),
                ),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/video_1/content"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"MP4".to_vec()))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/v1/videos/video_1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id": "video_1", "deleted": true})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let p = provider(&server);
        let SubmitOutcome::Async(handle) = p
            .submit(&request("sora-2", json!({"seconds": 4})))
            .await
            .unwrap()
        else {
            panic!("sync")
        };
        assert_eq!(handle.provider_request_id, "video_1");
        assert!(matches!(
            p.poll(&handle).await.unwrap(),
            PollStatus::InQueue { .. }
        ));
        let PollStatus::Completed(done) = p.poll(&handle).await.unwrap() else {
            panic!("not completed")
        };
        assert_eq!(done.model, "sora-2");
        let [Part::Video { url, mime }] = done.parts.as_slice() else {
            panic!("{:?}", done.parts)
        };
        assert_eq!(url, "data:video/mp4;base64,TVA0");
        assert_eq!(mime.as_deref(), Some("video/mp4"));
        p.cancel(&handle).await.unwrap();
    }

    #[tokio::test]
    async fn a_failed_sora_job_reports_openais_reason() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/video_2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "video_2", "status": "failed",
                "error": {"code": "moderation_blocked", "message": "blocked"}
            })))
            .mount(&server)
            .await;
        let handle = JobHandle {
            provider_request_id: "video_2".into(),
            poll_url: None,
            response_url: None,
        };
        let PollStatus::Failed(m) = provider(&server).poll(&handle).await.unwrap() else {
            panic!("not failed")
        };
        assert_eq!(m, "openai: blocked (moderation_blocked)");
    }
}
