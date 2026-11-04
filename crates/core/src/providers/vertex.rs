//! Vertex AI provider implementation.

use async_trait::async_trait;
use kalpa_libgen::vertex;
use tracing::{info, instrument};
use uuid::Uuid;

use crate::auth::{ServiceAccount, VertexAuthToken};
use crate::error::{KalpaError, KalpaResult};
use crate::generation::{GenerationRequest, GenerationResponse, Part};
use crate::provider::{
    CompletionProvider, GenerationProvider, ImageGenerationProvider, JobHandle, PollStatus,
    SubmitOutcome, VideoGenerationProvider,
};
use crate::types::{
    CompletionRequest, CompletionResponse, GeneratedImage, GeneratedVideo, ImageGenerationRequest,
    ImageGenerationResponse, Message, Role, Usage, VideoGenerationRequest, VideoGenerationResponse,
};
use std::path::Path;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;
use tokio::time::sleep;

struct AuthInner {
    token: VertexAuthToken,
    client: vertex::Client,
}

/// Vertex AI provider for Gemini models, Imagen, and Veo.
pub struct VertexProvider {
    /// Shared unauthenticated transport; each request attaches the current token.
    http: reqwest::Client,
    project_id: String,
    location: String,
    gcs_bucket: Option<String>,
    /// When set, access tokens are refreshed via JWT exchange before expiry.
    service_account: Option<ServiceAccount>,
    auth: Mutex<AuthInner>,
}

impl VertexProvider {
    fn build_client(bearer_token: &str) -> vertex::Client {
        let base_url = "https://aiplatform.googleapis.com";
        vertex::Client::new_with_client(
            base_url,
            crate::http::client_builder()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert(
                        reqwest::header::AUTHORIZATION,
                        crate::http::secret_header(&format!("Bearer {bearer_token}")),
                    );
                    headers
                })
                .build()
                .unwrap(),
        )
    }

    /// Create a provider from a static OAuth access token (no refresh).
    ///
    /// Prefer [`Self::from_service_account`] for long-lived processes — Google
    /// access tokens expire in ~1 hour.
    pub fn new(
        bearer_token: String,
        project_id: String,
        _location: String,
        gcs_bucket: Option<String>,
    ) -> Self {
        let client = Self::build_client(&bearer_token);
        let token = VertexAuthToken {
            access_token: bearer_token,
            // Unused when `service_account` is None (no refresh).
            expires_at: SystemTime::now() + Duration::from_secs(3600),
            project_id: project_id.clone(),
        };

        Self {
            http: crate::http::client_builder()
                .build()
                .expect("HTTP client initialization"),
            project_id,
            location: "global".to_string(),
            gcs_bucket,
            service_account: None,
            auth: Mutex::new(AuthInner { token, client }),
        }
    }

    /// Create a provider that mints and refreshes OAuth access tokens from a service account.
    pub async fn from_service_account(
        sa: ServiceAccount,
        _location: String,
        gcs_bucket: Option<String>,
    ) -> KalpaResult<Self> {
        let token = VertexAuthToken::from_service_account(sa.clone()).await?;
        let project_id = token.project_id.clone();
        let client = Self::build_client(&token.access_token);

        Ok(Self {
            http: crate::http::client_builder().build()?,
            project_id,
            location: "global".to_string(),
            gcs_bucket,
            service_account: Some(sa),
            auth: Mutex::new(AuthInner { token, client }),
        })
    }

    /// Load a service-account JSON file and build a refreshable provider.
    pub async fn from_service_account_file<P: AsRef<Path>>(
        path: P,
        location: String,
        gcs_bucket: Option<String>,
    ) -> KalpaResult<Self> {
        let json_content = tokio::fs::read_to_string(path.as_ref())
            .await
            .map_err(|e| {
                KalpaError::Config(format!(
                    "Failed to read service account file {}: {}",
                    path.as_ref().display(),
                    e
                ))
            })?;
        let sa: ServiceAccount = serde_json::from_str(&json_content)
            .map_err(|e| KalpaError::Config(format!("Invalid service account JSON: {}", e)))?;
        Self::from_service_account(sa, location, gcs_bucket).await
    }

    /// Ensure the access token is fresh, then return a cloned client and bearer.
    async fn auth_snapshot(&self) -> KalpaResult<(vertex::Client, String)> {
        let mut inner = self.auth.lock().await;
        if let Some(sa) = &self.service_account {
            if inner.token.is_expired() {
                inner.token.refresh_if_needed_sa(sa).await?;
                inner.client = Self::build_client(&inner.token.access_token);
                info!("refreshed Vertex AI OAuth access token");
            }
        }
        Ok((inner.client.clone(), inner.token.access_token.clone()))
    }

    /// Google Cloud project ID for this provider.
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// Current OAuth access token, refreshing first when SA-backed.
    pub async fn access_token(&self) -> KalpaResult<String> {
        Ok(self.auth_snapshot().await?.1)
    }

    /// Convert kalpa Message to Vertex Content
    fn convert_message_to_content(message: &Message) -> vertex::types::Content {
        let role = match message.role {
            Role::User => "user".to_string(),
            Role::Assistant => "model".to_string(),
            Role::System => "user".to_string(), // System messages treated as user
        };

        let parts = vec![vertex::types::Part {
            text: Some(message.content.clone()),
            inline_data: None,
            file_data: None,
        }];

        vertex::types::Content {
            role: Some(role),
            parts,
        }
    }

    /// Convert Vertex Content to kalpa Message
    fn convert_content_to_message(content: &vertex::types::Content) -> Option<Message> {
        let role = match content.role.as_deref() {
            Some("model") => Role::Assistant,
            Some("user") => Role::User,
            _ => Role::Assistant,
        };

        let text = content
            .parts
            .iter()
            .filter_map(|p| p.text.clone())
            .collect::<Vec<_>>()
            .join("\n");

        Some(Message {
            role,
            content: text,
        })
    }
}

#[async_trait]
impl CompletionProvider for VertexProvider {
    fn name(&self) -> &str {
        "vertex"
    }

    fn supported_models(&self) -> &[&str] {
        &[
            // Latest Gemini models
            "gemini-3.1-flash", // Nano Banana 2
            "gemini-3-pro",     // Nano Banana Pro
            "gemini-2.5-flash", // Original Nano Banana
            // Gemini 2.0 series
            "gemini-2.0-flash",
            "gemini-2.0-flash-exp",
            // Gemini 1.5 series
            "gemini-1.5-pro",
            "gemini-1.5-flash",
        ]
    }

    #[instrument(skip(self, request), fields(model = %request.model, provider = "vertex"))]
    async fn complete(&self, request: &CompletionRequest) -> KalpaResult<CompletionResponse> {
        info!(model = %request.model, "Sending completion request to Vertex AI");
        let (client, _) = self.auth_snapshot().await?;

        // Convert messages to Vertex format
        let contents: Vec<_> = request
            .messages
            .iter()
            .map(Self::convert_message_to_content)
            .collect();

        // Build generation config
        let generation_config = Some(vertex::types::GenerationConfig {
            temperature: request.temperature.map(f64::from),
            max_output_tokens: request.max_tokens.map(|t| t as i64),
            top_p: request.top_p.map(f64::from),
            top_k: None,
            stop_sequences: request.stop_sequences.clone().unwrap_or_default(),
            response_mime_type: None,
            response_modalities: Vec::new(),
        });

        let vertex_request = vertex::types::GenerateRequest {
            contents,
            generation_config,
            safety_settings: Vec::new(),
        };

        // Make the API call
        let response = client
            .generate_content(
                &self.project_id,
                &self.location,
                &request.model,
                &vertex_request,
            )
            .await
            .map_err(crate::http::generated_error)?;

        // Extract the response
        let candidate = response
            .candidates
            .first()
            .ok_or_else(|| KalpaError::ProviderError {
                status: 500,
                message: "No candidates in response".to_string(),
            })?;

        let content_text = candidate
            .content
            .as_ref()
            .and_then(|c| Self::convert_content_to_message(c))
            .ok_or_else(|| KalpaError::ProviderError {
                status: 500,
                message: "No content in candidate".to_string(),
            })?
            .content;

        let usage = response.usage_metadata.as_ref().map(|u| Usage {
            prompt_tokens: u.prompt_token_count.unwrap_or(0) as u32,
            completion_tokens: u.candidates_token_count.unwrap_or(0) as u32,
            total_tokens: u.total_token_count.unwrap_or(0) as u32,
        });

        Ok(CompletionResponse {
            content: content_text,
            model: request.model.clone(),
            usage,
            finish_reason: candidate.finish_reason.clone(),
        })
    }

    async fn health_check(&self) -> KalpaResult<()> {
        // Simple health check - try to generate with minimal tokens
        let request = CompletionRequest {
            model: "gemini-2.0-flash".to_string(),
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
impl ImageGenerationProvider for VertexProvider {
    fn name(&self) -> &str {
        "vertex"
    }

    fn supported_models(&self) -> &[&str] {
        &[
            // Gemini image models (use :generateContent with responseModalities)
            "gemini-3.1-flash-image", // Nano Banana 2
            "gemini-3-pro-image",     // Nano Banana Pro
            "gemini-2.5-flash-image", // Nano Banana (original)
            // Imagen 4.0 (latest)
            "imagen-4.0-generate-001",
            // Imagen 3.0
            "imagen-3.0-generate-001",
            "imagen-3.0-generate-002",
            "imagen-3.0-fast-generate-001",
        ]
    }

    async fn generate_image(
        &self,
        request: &ImageGenerationRequest,
    ) -> KalpaResult<ImageGenerationResponse> {
        // Gemini image models use :generateContent with responseModalities
        if request.model.starts_with("gemini-") {
            return self
                .generate_image_gemini(&request.model, &request.prompt)
                .await;
        }

        // Imagen models use :predict format
        let instances = vec![vertex::types::ImageInstance {
            prompt: request.prompt.clone(),
        }];

        let parameters = Some(vertex::types::ImageParameters {
            sample_count: std::num::NonZeroU64::new(1),
            aspect_ratio: None,
            negative_prompt: None,
        });

        let predict_request = vertex::types::PredictRequest {
            instances,
            parameters,
        };

        // Make the API call using :predict endpoint (required for Imagen models)
        let (client, _) = self.auth_snapshot().await?;
        let response = client
            .predict(
                &self.project_id,
                &self.location,
                &request.model,
                &predict_request,
            )
            .await
            .map_err(crate::http::generated_error)?;

        // Extract image data from response
        let images: Vec<GeneratedImage> = if let Some(predictions) = &response.predictions {
            predictions
                .iter()
                .filter_map(|prediction| {
                    prediction
                        .bytes_base64_encoded
                        .as_ref()
                        .map(|data| GeneratedImage {
                            url: None,
                            b64_data: Some(data.clone()),
                        })
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(ImageGenerationResponse {
            images,
            model: request.model.clone(),
        })
    }
}

#[async_trait]
impl VideoGenerationProvider for VertexProvider {
    fn name(&self) -> &str {
        "vertex"
    }

    fn supported_models(&self) -> &[&str] {
        &[
            // Veo 3.0
            "veo-3.0-generate",
            "veo-3.0-fast-generate-preview",
            // Veo 2.0
            "veo-2.0-generate-001",
        ]
    }

    async fn generate_video(
        &self,
        request: &VideoGenerationRequest,
    ) -> KalpaResult<VideoGenerationResponse> {
        // STEP 1: Generate request_id BEFORE sending the generation request
        let request_id = Uuid::new_v4().to_string();

        // Build video generation request using VideoPredictRequest format
        let video_instance = vertex::types::VideoInstance {
            prompt: request.prompt.clone(),
            image: None,
        };

        // STEP 2: Veo 2.0 requires a GCS storageUri with request_id for output videos
        // Extract bucket name from configured gcs_bucket
        let fallback_bucket;
        let bucket_name = if let Some(gcs_uri) = &self.gcs_bucket {
            // Parse "gs://bucket-name/path" to get "bucket-name"
            gcs_uri
                .strip_prefix("gs://")
                .and_then(|s| s.split('/').next())
                .ok_or_else(|| KalpaError::ProviderError {
                    status: 500,
                    message: format!("Invalid GCS bucket URI format: {}", gcs_uri),
                })?
        } else {
            // Fallback bucket name
            fallback_bucket = format!("{}-kalpa-videos", self.project_id);
            fallback_bucket.as_str()
        };

        // Construct storage URI with request_id
        let storage_uri = format!("gs://{}/generations/{}/", bucket_name, request_id);

        let parameters = Some(vertex::types::VideoParameters {
            sample_count: Some(1),
            aspect_ratio: None,
            storage_uri: Some(storage_uri),
        });

        let video_request = vertex::types::VideoPredictRequest {
            instances: vec![video_instance],
            parameters,
        };

        // Use predictLongRunning endpoint (required for Veo models)
        let (client, _) = self.auth_snapshot().await?;
        let response = client
            .predict_long_running(
                &self.project_id,
                &self.location,
                &request.model,
                &video_request,
            )
            .await
            .map_err(crate::http::generated_error)?;

        // Extract operation response
        let operation_response = response.into_inner();

        // Check if this is a long-running operation
        if let Some(op_name) = &operation_response.name {
            // Poll for operation completion
            let final_response = self.poll_operation(op_name).await?;

            // STEP 3: Extract video data from completed operation using request_id
            let videos = self
                .extract_videos_from_operation(&final_response, &request_id, bucket_name)
                .await?;

            Ok(VideoGenerationResponse {
                videos,
                model: request.model.clone(),
            })
        } else {
            // Immediate response (unlikely for video generation)
            return Err(KalpaError::ProviderError {
                status: 500,
                message: "Unexpected response format: expected long-running operation".to_string(),
            });
        }
    }
}

impl VertexProvider {
    /// Generate an image using a Gemini image model via :generateContent with responseModalities.
    ///
    /// Models like `gemini-3.1-flash-image` and `gemini-3-pro-image` use the standard
    /// `generateContent` endpoint but with `generationConfig.responseModalities: ["IMAGE", "TEXT"]`
    /// to produce inline base64 images in the response.
    async fn generate_image_gemini(
        &self,
        model: &str,
        prompt: &str,
    ) -> KalpaResult<ImageGenerationResponse> {
        // Build request using libgen types — same endpoint as text but with responseModalities
        let contents = vec![vertex::types::Content {
            role: Some("user".to_string()),
            parts: vec![vertex::types::Part {
                text: Some(prompt.to_string()),
                inline_data: None,
                file_data: None,
            }],
        }];

        let generation_config = Some(vertex::types::GenerationConfig {
            temperature: None,
            max_output_tokens: None,
            top_p: None,
            top_k: None,
            stop_sequences: Vec::new(),
            response_mime_type: None,
            response_modalities: vec!["IMAGE".to_string(), "TEXT".to_string()],
        });

        let vertex_request = vertex::types::GenerateRequest {
            contents,
            generation_config,
            safety_settings: Vec::new(),
        };

        // Call :generateContent via the libgen client
        let (client, _) = self.auth_snapshot().await?;
        let response = client
            .generate_content(&self.project_id, &self.location, model, &vertex_request)
            .await
            .map_err(crate::http::generated_error)?;

        // Extract images from candidates[].content.parts[].inlineData
        let mut images = Vec::new();
        for candidate in &response.candidates {
            if let Some(content) = &candidate.content {
                for part in &content.parts {
                    if let Some(data) = &part.inline_data {
                        if data.mime_type.starts_with("image/") {
                            images.push(GeneratedImage {
                                url: None,
                                b64_data: Some(data.data.clone()),
                            });
                        }
                    }
                }
            }
        }

        if images.is_empty() {
            return Err(KalpaError::ProviderError {
                status: 500,
                message: "No images returned from Gemini model response".to_string(),
            });
        }

        Ok(ImageGenerationResponse {
            images,
            model: model.to_string(),
        })
    }

    /// List objects in a GCS bucket with a given prefix using the GCS JSON REST API
    async fn list_gcs_objects(
        &self,
        bucket_name: &str,
        prefix: &str,
    ) -> KalpaResult<Vec<GcsObject>> {
        let url = format!(
            "https://storage.googleapis.com/storage/v1/b/{}/o?prefix={}",
            bucket_name, prefix
        );

        let (_, bearer_token) = self.auth_snapshot().await?;
        let response = self
            .http
            .get(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                crate::http::secret_header(&format!("Bearer {bearer_token}")),
            )
            .send()
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("Failed to list GCS objects: {}", e),
            })?;

        let response = crate::http::check_response(response, "GCS listing").await?;
        let gcs_response: GcsListResponse = crate::http::read_json(response).await?;

        Ok(gcs_response.items.unwrap_or_default())
    }

    /// Poll a long-running operation until completion
    async fn poll_operation(
        &self,
        operation_name: &str,
    ) -> KalpaResult<vertex::types::OperationResponse> {
        tokio::time::timeout(
            Duration::from_secs(600),
            self.poll_operation_inner(operation_name),
        )
        .await
        .map_err(|_| KalpaError::ProviderError {
            status: 504,
            message: "Vertex operation deadline exceeded".into(),
        })?
    }

    async fn poll_operation_inner(
        &self,
        operation_name: &str,
    ) -> KalpaResult<vertex::types::OperationResponse> {
        let max_attempts = 120;
        let poll_interval = Duration::from_secs(5);

        // Extract model name from the operation_name
        // Format: "projects/.../locations/.../publishers/.../models/{model}/operations/{uuid}"
        let model_name = operation_name
            .split("/models/")
            .nth(1)
            .and_then(|s| s.split("/operations/").next())
            .ok_or_else(|| KalpaError::ProviderError {
                status: 500,
                message: format!("Invalid operation name format: {}", operation_name),
            })?;

        for attempt in 1..=max_attempts {
            // Wait before polling
            if attempt > 1 {
                sleep(poll_interval).await;
            }

            // Veo operations use a special :fetchPredictOperation endpoint
            // This is NOT the standard LRO operations API
            let url = format!(
                "https://aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:fetchPredictOperation",
                self.project_id, self.location, model_name
            );

            // The request body must contain only the operation name
            // The GCS URI is already in the response since we passed storageUri in the initial request
            let body = serde_json::json!({
                "operationName": operation_name
            });

            let (_, bearer_token) = self.auth_snapshot().await?;
            let http_response = self
                .http
                .post(&url)
                .header(
                    reqwest::header::AUTHORIZATION,
                    crate::http::secret_header(&format!("Bearer {bearer_token}")),
                )
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| KalpaError::ProviderError {
                    status: 500,
                    message: format!("Failed to send poll request: {}", e),
                })?;

            let response = crate::http::check_response(http_response, "Vertex operation").await?;
            let operation_response: vertex::types::OperationResponse =
                crate::http::read_json(response).await?;

            // Check if operation is done
            if operation_response.done.unwrap_or(false) {
                // Check for errors
                if let Some(error) = &operation_response.error {
                    return Err(KalpaError::ProviderError {
                        status: error.code.unwrap_or(500) as u16,
                        message: format!(
                            "Video generation failed: {}",
                            error.message.as_deref().unwrap_or("Unknown error")
                        ),
                    });
                }

                return Ok(operation_response);
            }
        }

        Err(KalpaError::ProviderError {
            status: 408,
            message: "Video generation timed out after 10 minutes".to_string(),
        })
    }

    /// Extract videos from operation response or GCS bucket using request_id
    async fn extract_videos_from_operation(
        &self,
        response: &vertex::types::OperationResponse,
        request_id: &str,
        bucket_name: &str,
    ) -> KalpaResult<Vec<GeneratedVideo>> {
        // Parse the response as raw JSON to handle the nested structure
        let response_json =
            serde_json::to_value(response).map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("Failed to serialize response: {}", e),
            })?;

        // Try to extract GCS URIs from response.generateVideoResponse.generatedSamples[].video.uri
        if let Some(response_obj) = response_json.get("response") {
            // Check for generateVideoResponse (Veo 2.0 format)
            if let Some(video_response) = response_obj.get("generateVideoResponse") {
                if let Some(samples) = video_response
                    .get("generatedSamples")
                    .and_then(|s| s.as_array())
                {
                    let videos: Vec<GeneratedVideo> = samples
                        .iter()
                        .filter_map(|sample| {
                            sample
                                .get("video")
                                .and_then(|v| v.get("uri"))
                                .and_then(|u| u.as_str())
                                .map(|uri| GeneratedVideo {
                                    url: uri.to_string(),
                                })
                        })
                        .collect();

                    if !videos.is_empty() {
                        return Ok(videos);
                    }
                }
            }

            // Fall back to checking predictions (older format or imagen)
            if let Some(predictions) = response
                .response
                .get("predictions")
                .and_then(|value| value.as_array())
            {
                let videos: Vec<_> = predictions
                    .iter()
                    .filter_map(|prediction| {
                        prediction
                            .get("bytesBase64Encoded")
                            .and_then(|value| value.as_str())
                            .map(|data| GeneratedVideo {
                                url: format!("data:video/mp4;base64,{data}"),
                            })
                    })
                    .collect();
                if !videos.is_empty() {
                    return Ok(videos);
                }
            }
        }

        // Response is empty - list GCS bucket using request_id to find the generated video
        let prefix = format!("generations/{}/", request_id);
        let objects = self.list_gcs_objects(bucket_name, &prefix).await?;

        // Filter for .mp4 files and find the most recently created one
        let video = objects
            .into_iter()
            .filter(|obj| obj.name.ends_with(".mp4"))
            .max_by_key(|obj| obj.time_created.clone())
            .ok_or_else(|| KalpaError::ProviderError {
                status: 500,
                message: format!("No video files found in GCS with prefix: {}", prefix),
            })?;

        let video_uri = format!("gs://{}/{}", bucket_name, video.name);

        Ok(vec![GeneratedVideo { url: video_uri }])
    }
}

/// GCS list response structure
#[derive(Debug, serde::Deserialize)]
struct GcsListResponse {
    items: Option<Vec<GcsObject>>,
}

/// GCS object metadata
#[derive(Debug, serde::Deserialize)]
struct GcsObject {
    name: String,
    #[serde(rename = "timeCreated")]
    time_created: String,
}

// ─── Unified GenerationProvider ──────────────────────────────────────────────
//
// Routes by model family: Imagen (`imagen-*`) → image `:predict` (b64 parts);
// otherwise Gemini text via `generateContent` → text parts. Both are
// synchronous from the caller's view. Veo (video) stays on the legacy
// VideoGenerationProvider for now.

#[async_trait]
impl GenerationProvider for VertexProvider {
    fn name(&self) -> &str {
        "vertex"
    }

    async fn submit(&self, request: &GenerationRequest) -> KalpaResult<SubmitOutcome> {
        let model = request.model.split().1.to_string();
        let prompt = request.text_prompt()?;

        if model.contains("imagen") {
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
            Ok(SubmitOutcome::Sync(GenerationResponse {
                model,
                parts,
                usage: None,
            }))
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

    async fn poll(&self, _handle: &JobHandle) -> KalpaResult<PollStatus> {
        Err(KalpaError::Other(
            "vertex generation is synchronous; poll not supported".into(),
        ))
    }
}

#[cfg(test)]
mod auth_tests {
    use super::*;

    fn invalid_service_account() -> ServiceAccount {
        ServiceAccount {
            account_type: "service_account".into(),
            project_id: "test-project".into(),
            private_key_id: "test-key-id".into(),
            private_key: "not-a-private-key".into(),
            client_email: "test@example.invalid".into(),
            client_id: "test-client".into(),
            auth_uri: String::new(),
            token_uri: "https://untrusted.example/token".into(),
        }
    }

    #[tokio::test]
    async fn fresh_service_account_token_is_reused_by_concurrent_callers() {
        let mut provider = VertexProvider::new(
            "cached-token".into(),
            "test-project".into(),
            "global".into(),
            None,
        );
        provider.service_account = Some(invalid_service_account());
        let (first, second) = tokio::join!(provider.access_token(), provider.access_token());
        assert_eq!(first.unwrap(), "cached-token");
        assert_eq!(second.unwrap(), "cached-token");
        assert_eq!(provider.project_id(), "test-project");
    }

    #[tokio::test]
    async fn failed_refresh_does_not_replace_cached_token() {
        let mut provider = VertexProvider::new(
            "cached-token".into(),
            "test-project".into(),
            "global".into(),
            None,
        );
        provider.service_account = Some(invalid_service_account());
        provider.auth.lock().await.token.expires_at = SystemTime::UNIX_EPOCH;
        assert!(provider.access_token().await.is_err());
        assert_eq!(
            provider.auth.lock().await.token.access_token,
            "cached-token"
        );
    }
}
