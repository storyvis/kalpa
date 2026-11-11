//! Fal AI provider implementation.
//!
//! Supports various Fal AI models for text-to-image, image-to-video, and text-to-video generation.
//! Video generation can use direct API or queue-based API depending on the model.

use async_trait::async_trait;
use kalpa_libgen::falai;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::{KalpaError, KalpaResult};
use crate::generation::{GenerationRequest, GenerationResponse, Part};
use crate::provider::{
    GenerationProvider, ImageGenerationProvider, JobHandle, PollStatus, SubmitOutcome,
    VideoGenerationProvider,
};
use crate::types::{
    GeneratedImage, GeneratedVideo, ImageGenerationRequest, ImageGenerationResponse,
    VideoGenerationRequest, VideoGenerationResponse,
};

// ─── Fal-specific wire types ─────────────────────────────────────────────────
//
// These live here (not in `types.rs`) because they are specific to the fal.ai
// queue protocol. They are re-exported from `crate::types` for backward compat.

/// Queue submission response from fal.ai
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalQueueSubmitResponse {
    /// Unique request ID for tracking
    pub request_id: String,
    /// URL to check status
    pub status_url: String,
    /// URL to get final result
    pub response_url: String,
    /// URL to cancel the request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_url: Option<String>,
}

/// Queue status response from fal.ai
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum FalQueueStatus {
    #[serde(rename = "IN_QUEUE")]
    InQueue {
        queue_position: Option<u32>,
        #[serde(default, deserialize_with = "deserialize_null_as_default")]
        logs: Vec<FalLogEntry>,
    },
    #[serde(rename = "IN_PROGRESS")]
    InProgress {
        #[serde(default, deserialize_with = "deserialize_null_as_default")]
        logs: Vec<FalLogEntry>,
    },
    #[serde(rename = "COMPLETED")]
    Completed {
        #[serde(default, deserialize_with = "deserialize_null_as_default")]
        logs: Vec<FalLogEntry>,
        response_url: Option<String>,
    },
    #[serde(rename = "FAILED")]
    Failed {
        error: String,
        #[serde(default, deserialize_with = "deserialize_null_as_default")]
        logs: Vec<FalLogEntry>,
    },
}

/// Log entry from fal.ai queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalLogEntry {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// Helper to deserialize null as default (empty vec)
fn deserialize_null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// Fal AI provider for various generative models.
#[derive(Clone)]
pub struct FalAIProvider {
    client: falai::Client,
    http_client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct FalAITextToImageRequest {
    prompt: String,
    /// A fal preset (`square_hd`, `landscape_16_9`, …) or `{width, height}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    image_size: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_images: Option<i32>,
    /// The image an edit model (image-to-image, Kontext, …) works from.
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<String>,
}

/// fal's `image_size`: an OpenAI-style `WxH` becomes `{width, height}`;
/// anything else is passed as a preset name.
fn fal_image_size(size: &str) -> serde_json::Value {
    let parsed = size
        .split_once(['x', 'X'])
        .and_then(|(w, h)| Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?)));
    match parsed {
        Some((width, height)) => serde_json::json!({ "width": width, "height": height }),
        None => serde_json::Value::String(size.to_string()),
    }
}

/// fal's image-to-video endpoint for a video model, by its naming
/// convention: `…/text-to-video` becomes `…/image-to-video`, and a bare
/// model (`fal-ai/minimax/video-01`) gains `/image-to-video`. An endpoint
/// already taking an image is kept. A model with no such sibling is refused
/// by fal (404, with its reason), never silently run without the image.
fn image_to_video_endpoint(model_id: &str) -> String {
    if model_id.contains("image-to-video") {
        model_id.to_string()
    } else if let Some(base) = model_id.strip_suffix("text-to-video") {
        format!("{base}image-to-video")
    } else {
        format!("{}/image-to-video", model_id.trim_end_matches('/'))
    }
}

/// The request's input image as fal takes it: its URL, or inline bytes as a
/// `data:` URI (fal accepts both). `params.image_url` is the fallback.
fn input_image(request: &GenerationRequest) -> Option<String> {
    request
        .parts
        .iter()
        .find_map(|p| match p {
            Part::Image { url: Some(u), .. } => Some(u.clone()),
            Part::Image {
                b64_data: Some(b64),
                mime,
                ..
            } => Some(format!(
                "data:{};base64,{b64}",
                mime.as_deref().unwrap_or("image/png")
            )),
            _ => None,
        })
        .or_else(|| {
            request
                .params
                .as_ref()
                .and_then(|p| p.get("image_url"))
                .and_then(|v| v.as_str())
                .map(String::from)
        })
}

#[derive(Debug, Serialize)]
struct FalAITextToVideoRequest {
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<i32>,
}

#[derive(Debug, Serialize)]
struct FalAIImageToVideoRequest {
    image_url: String,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct FalAIImageResponse {
    images: Vec<FalAIImage>,
}

#[derive(Debug, Deserialize)]
struct FalAIImage {
    url: String,
    #[serde(default)]
    width: Option<i32>,
    #[serde(default)]
    height: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FalAIVideoResponse {
    #[serde(alias = "video")]
    #[serde(default)]
    video: Option<FalAIVideo>,
    #[serde(default)]
    videos: Vec<FalAIVideo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FalAIVideo {
    url: String,
}

impl FalAIProvider {
    /// Create a new Fal AI provider.
    ///
    /// # Arguments
    /// * `api_key` - Fal AI API key
    pub fn new(api_key: String) -> Self {
        // Create reqwest client with authentication headers
        let reqwest_client = crate::http::client_builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    crate::http::secret_header(&format!("Key {}", api_key)),
                );
                headers
            })
            .timeout(Duration::from_secs(300)) // 5 minute timeout
            .build()
            .unwrap();

        // Create progenitor-generated client using the queue.fal.run base URL
        let client =
            falai::Client::new_with_client("https://queue.fal.run", reqwest_client.clone());

        Self {
            client,
            http_client: reqwest_client,
        }
    }

    /// Upload a file to Fal.ai storage and return the URL (or data URL)
    ///
    /// # Arguments
    /// * `file_path` - Path to the local file to upload
    ///
    /// # Returns
    /// * `KalpaResult<String>` - URL of the uploaded file (data URL as fallback)
    pub async fn upload_file(&self, file_path: &str) -> KalpaResult<String> {
        use std::path::Path;

        // Read the file
        let file_data =
            tokio::fs::read(file_path)
                .await
                .map_err(|e| KalpaError::ProviderError {
                    status: 400u16,
                    message: format!("Failed to read file: {}", e),
                })?;

        // Detect content type from extension
        let content_type = match Path::new(file_path).extension().and_then(|e| e.to_str()) {
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("png") => "image/png",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            _ => "image/jpeg",
        };

        // Base64 encode the file and create a data URL
        // Fal.ai accepts data URLs directly in API requests
        use base64::Engine;
        let base64_data = base64::engine::general_purpose::STANDARD.encode(&file_data);
        let data_url = format!("data:{};base64,{}", content_type, base64_data);

        Ok(data_url)
    }

    /// Submit a request to fal.ai queue using progenitor-generated client
    pub async fn queue_submit<T>(
        &self,
        model_id: &str,
        request: &T,
    ) -> KalpaResult<FalQueueSubmitResponse>
    where
        T: Serialize,
    {
        validate_model_path(model_id)?;
        // Serialize request to serde_json::Value and convert to Map
        let request_value =
            serde_json::to_value(request).map_err(|e| KalpaError::ProviderError {
                status: 400u16,
                message: format!("Failed to serialize request: {}", e),
            })?;

        let request_body = request_value
            .as_object()
            .ok_or_else(|| KalpaError::ProviderError {
                status: 400u16,
                message: "Request must be a JSON object".to_string(),
            })?;

        // Use progenitor-generated method
        let response = self
            .client
            .submit_to_queue(model_id, request_body)
            .await
            .map_err(crate::http::generated_error)?
            .into_inner();

        // Convert from generated type to our custom type
        Ok(FalQueueSubmitResponse {
            request_id: response.request_id,
            status_url: response
                .status_url
                .ok_or_else(|| KalpaError::ProviderError {
                    status: 502,
                    message: "Fal submit response missing status URL".into(),
                })?,
            response_url: response
                .response_url
                .ok_or_else(|| KalpaError::ProviderError {
                    status: 502,
                    message: "Fal submit response missing result URL".into(),
                })?,
            cancel_url: response.cancel_url,
        })
    }

    /// Check status of a queued request using progenitor-generated client
    pub async fn queue_status(
        &self,
        model_id: &str,
        request_id: &str,
    ) -> KalpaResult<FalQueueStatus> {
        validate_model_path(model_id)?;
        // Use progenitor-generated method
        let response = self
            .client
            .get_queue_status(model_id, request_id, Some("1"))
            .await
            .map_err(crate::http::generated_error)?
            .into_inner();

        // Convert from generated response to our custom enum via JSON serialization
        let status_json =
            serde_json::to_value(&response).map_err(|e| KalpaError::ProviderError {
                status: 500u16,
                message: format!("Failed to serialize status response: {}", e),
            })?;

        let status: FalQueueStatus =
            serde_json::from_value(status_json).map_err(|_| KalpaError::ProviderError {
                status: 500u16,
                message: "Invalid Fal queue status response".into(),
            })?;

        Ok(status)
    }

    /// Check status using a direct status URL (preferred over queue_status).
    /// Fal.ai returns a canonical status_url in the submit response.
    /// Makes a direct GET request to the status URL with authentication.
    pub async fn queue_status_by_url(&self, status_url: &str) -> KalpaResult<FalQueueStatus> {
        let mut url = Self::trusted_queue_url(status_url)?;
        url.query_pairs_mut().append_pair("logs", "1");
        let response = self.http_client.get(url).send().await?;
        crate::http::read_json(crate::http::check_response(response, "fal").await?).await
    }

    fn trusted_queue_url(value: &str) -> KalpaResult<reqwest::Url> {
        let url = reqwest::Url::parse(value)
            .map_err(|_| KalpaError::Config("Invalid Fal queue URL".into()))?;
        if url.scheme() != "https"
            || url.host_str() != Some("queue.fal.run")
            || url.port_or_known_default() != Some(443)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(KalpaError::Config(
                "Fal queue URL must use https://queue.fal.run".into(),
            ));
        }
        Ok(url)
    }

    /// Parse a fal.ai response URL to extract model_id and request_id.
    /// Expected format: https://queue.fal.run/{model_id}/requests/{request_id}
    fn parse_response_url(response_url: &str) -> KalpaResult<(String, String)> {
        let url = Self::trusted_queue_url(response_url)?;
        let path = url.path().trim_start_matches('/');
        let (model_id, request_id) = path
            .split_once("/requests/")
            .ok_or_else(|| KalpaError::Config("Invalid Fal result URL path".into()))?;
        validate_model_path(model_id)?;
        if request_id.is_empty()
            || !request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(KalpaError::Config("Invalid Fal request ID".into()));
        }
        Ok((model_id.into(), request_id.into()))
    }

    /// Get the result of a completed queue request.
    /// Returns the raw JSON value of the result.
    pub async fn queue_result(
        &self,
        model_id: &str,
        request_id: &str,
    ) -> KalpaResult<serde_json::Value> {
        validate_model_path(model_id)?;
        let result = self
            .client
            .get_queue_result(model_id, request_id)
            .await
            .map_err(crate::http::generated_error)?
            .into_inner();

        serde_json::to_value(&result).map_err(|e| KalpaError::ProviderError {
            status: 500u16,
            message: format!("Failed to serialize result: {}", e),
        })
    }

    /// Get the result of a completed queue request using the canonical response URL.
    /// Parses the URL to extract model_id and request_id, then delegates to queue_result.
    /// Fetched directly (not through the generated client) so a failed
    /// result, such as a 422 for invalid input, keeps fal's reason.
    pub async fn queue_result_by_url(&self, response_url: &str) -> KalpaResult<serde_json::Value> {
        Self::parse_response_url(response_url)?;
        let url = Self::trusted_queue_url(response_url)?;
        let response = self.http_client.get(url).send().await?;
        crate::http::read_json(crate::http::check_response(response, "fal").await?).await
    }

    /// Build and serialize a video request body for queue submission.
    /// This is useful for the CLI to submit requests directly.
    pub fn build_video_request_body(
        &self,
        model: &str,
        prompt: &str,
        image_url: Option<&str>,
        duration: Option<i32>,
    ) -> KalpaResult<serde_json::Value> {
        if model.contains("image-to-video") {
            let image_url = image_url.ok_or_else(|| KalpaError::ProviderError {
                status: 400u16,
                message: "image_url required for image-to-video models".to_string(),
            })?;
            let req = FalAIImageToVideoRequest {
                image_url: image_url.to_string(),
                prompt: prompt.to_string(),
                duration,
            };
            serde_json::to_value(&req).map_err(|e| KalpaError::ProviderError {
                status: 400u16,
                message: format!("Failed to serialize request: {}", e),
            })
        } else {
            let req = FalAITextToVideoRequest {
                prompt: prompt.to_string(),
                duration,
            };
            serde_json::to_value(&req).map_err(|e| KalpaError::ProviderError {
                status: 400u16,
                message: format!("Failed to serialize request: {}", e),
            })
        }
    }

    /// Submit and wait for queue completion (with polling).
    /// Uses the canonical status_url and response_url from the submit response via
    /// direct GET requests, since fal.ai uses shortened model paths in those URLs.
    async fn queue_submit_and_wait<T, R>(&self, model_id: &str, request: &T) -> KalpaResult<R>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        tokio::time::timeout(
            Duration::from_secs(900),
            self.queue_submit_and_wait_inner(model_id, request),
        )
        .await
        .map_err(|_| KalpaError::ProviderError {
            status: 504,
            message: "Fal generation deadline exceeded".into(),
        })?
    }

    async fn queue_submit_and_wait_inner<T, R>(&self, model_id: &str, request: &T) -> KalpaResult<R>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        let submit_response = self.queue_submit(model_id, request).await?;
        let mut cancellation = crate::provider::CancelOnDrop {
            provider: std::sync::Arc::new(self.clone()),
            handle: Some(JobHandle {
                provider_request_id: submit_response.request_id.clone(),
                poll_url: Some(submit_response.status_url.clone()),
                response_url: Some(submit_response.response_url.clone()),
            }),
        };

        // Use the canonical URLs from the submit response for polling
        let status_url = submit_response.status_url.clone();
        let response_url = submit_response.response_url.clone();

        // Poll for completion using direct GET to the status_url
        loop {
            let status = self.queue_status_by_url(&status_url).await?;

            match status {
                FalQueueStatus::Completed { .. } => {
                    cancellation.handle = None;
                    // Fetch the actual result using direct GET to the response_url
                    let result_json = self.queue_result_by_url(&response_url).await?;

                    return serde_json::from_value(result_json).map_err(|_| {
                        KalpaError::ProviderError {
                            status: 500u16,
                            message: "Invalid Fal result response".into(),
                        }
                    });
                }
                FalQueueStatus::Failed { error: _, .. } => {
                    cancellation.handle = None;
                    return Err(KalpaError::ProviderError {
                        status: 500u16,
                        message: "Fal AI generation failed".into(),
                    });
                }
                FalQueueStatus::InQueue { .. } => {
                    // Queue position tracking
                }
                FalQueueStatus::InProgress { .. } => {
                    // Processing in progress
                }
            }

            // Wait before polling again
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

#[async_trait]
impl ImageGenerationProvider for FalAIProvider {
    fn name(&self) -> &str {
        "fal"
    }

    fn supported_models(&self) -> &[&str] {
        &[
            // Text-to-Image models
            "fal-ai/flux/dev",
            "fal-ai/flux/schnell",
            "fal-ai/flux-pro",
            "fal-ai/flux-realism",
            "fal-ai/recraft-v3",
            "fal-ai/aura-flow",
            "fal-ai/stable-diffusion-v3-medium",
            "fal-ai/fast-sdxl",
        ]
    }

    async fn generate_image(
        &self,
        request: &ImageGenerationRequest,
    ) -> KalpaResult<ImageGenerationResponse> {
        let fal_request = FalAITextToImageRequest {
            prompt: request.prompt.clone(),
            image_size: request.size.as_deref().map(fal_image_size),
            num_images: Some(1),
            image_url: None,
        };

        // Use queue-based API for all image generations
        let response: FalAIImageResponse = self
            .queue_submit_and_wait(&request.model, &fal_request)
            .await?;

        let images: Vec<GeneratedImage> = response
            .images
            .into_iter()
            .map(|img| GeneratedImage {
                url: Some(img.url),
                b64_data: None,
            })
            .collect();

        Ok(ImageGenerationResponse {
            images,
            model: request.model.clone(),
        })
    }
}

#[async_trait]
impl VideoGenerationProvider for FalAIProvider {
    fn name(&self) -> &str {
        "fal"
    }

    fn supported_models(&self) -> &[&str] {
        &[
            // Text-to-Video models
            "fal-ai/minimax/video-01",
            "fal-ai/minimax/video-01-live",
            "fal-ai/hunyuan-video",
            "fal-ai/mochi-v1",
            "fal-ai/kling-video/v1/standard/text-to-video",
            "fal-ai/kling-video/v1.5/standard/text-to-video",
            "fal-ai/kling-video/v1.6/standard/text-to-video",
            "fal-ai/kling-video/v2.1/master/text-to-video",
            "fal-ai/kling-video/v2.6/pro/text-to-video",
            "fal-ai/wan/v2.2-a14b/text-to-video",
            "fal-ai/ltx-2/text-to-video",
            "fal-ai/ltx-2.3/text-to-video",
            "fal-ai/veo3",
            "fal-ai/veo3.1",
            "bytedance/seedance-2.0/text-to-video",
            "bytedance/seedance-2.0/fast/text-to-video",
            // Image-to-Video models
            "fal-ai/veo2/image-to-video",
            "fal-ai/veo3/image-to-video",
            "fal-ai/luma-dream-machine/image-to-video",
            "fal-ai/kling-video/v2.1/master/image-to-video",
            "fal-ai/kling-video/v1.6/pro/image-to-video",
            "fal-ai/minimax/video-01-live/image-to-video",
            "fal-ai/pixverse/v4.5/image-to-video",
            "bytedance/seedance-2.0/image-to-video",
            // Legacy models
            "fal-ai/kling-video/v1/standard/image-to-video",
            "fal-ai/kling-video/v1.5/standard/image-to-video",
            "fal-ai/minimax/video-01/image-to-video",
            "fal-ai/wan/v2.2-a14b/image-to-video",
            "fal-ai/luma-dream-machine",
        ]
    }

    async fn generate_video(
        &self,
        request: &VideoGenerationRequest,
    ) -> KalpaResult<VideoGenerationResponse> {
        // Determine if this is text-to-video or image-to-video based on model and input
        let response: FalAIVideoResponse = if request.model.contains("image-to-video") {
            // Image-to-video
            let image_url =
                request
                    .image_url
                    .as_ref()
                    .ok_or_else(|| KalpaError::ProviderError {
                        status: 400u16,
                        message: "image_url required for image-to-video models".to_string(),
                    })?;

            let fal_request = FalAIImageToVideoRequest {
                image_url: image_url.clone(),
                prompt: request.prompt.clone(),
                duration: request.duration,
            };

            // Use queue-based API for all video generations
            self.queue_submit_and_wait(&request.model, &fal_request)
                .await?
        } else {
            // Text-to-video
            let fal_request = FalAITextToVideoRequest {
                prompt: request.prompt.clone(),
                duration: request.duration,
            };

            // Use queue-based API for all video generations
            self.queue_submit_and_wait(&request.model, &fal_request)
                .await?
        };

        // Handle both response formats: single video or videos array
        let videos = if let Some(video) = response.video {
            vec![GeneratedVideo { url: video.url }]
        } else if !response.videos.is_empty() {
            response
                .videos
                .into_iter()
                .map(|v| GeneratedVideo { url: v.url })
                .collect()
        } else {
            return Err(KalpaError::ProviderError {
                status: 502,
                message: "No video in Fal response".into(),
            });
        };

        Ok(VideoGenerationResponse {
            videos,
            model: request.model.clone(),
        })
    }
}

// ─── Unified GenerationProvider (text→image) ─────────────────────────────────
//
// Uses a direct queue submit/poll so the real HTTP status is visible — a `429`
// becomes `KalpaError::RateLimited`, which the limiter turns into a decrease.

impl FalAIProvider {
    /// Direct POST to the fal queue so we can read the HTTP status (429 detection).
    async fn submit_direct(
        &self,
        model_id: &str,
        body: &serde_json::Value,
    ) -> KalpaResult<FalQueueSubmitResponse> {
        validate_model_path(model_id)?;
        let resp = self
            .http_client
            .post(format!("https://queue.fal.run/{}", model_id))
            .json(body)
            .send()
            .await?;
        let resp = crate::http::check_response(resp, "fal").await?;
        crate::http::read_json(resp).await
    }
}

#[async_trait]
impl GenerationProvider for FalAIProvider {
    fn name(&self) -> &str {
        "fal"
    }

    async fn submit(&self, request: &GenerationRequest) -> KalpaResult<SubmitOutcome> {
        let model_id = request.model.split().1.to_string();
        let prompt = request.text_prompt()?;
        let duration = request
            .params
            .as_ref()
            .and_then(|p| p.get("duration"))
            .and_then(|v| v.as_i64())
            .map(|d| d as i32);

        // Branch on modality: video models take a video request body; an input
        // image (URL or inline) selects image-to-video, or for an image model
        // is the image an edit works from.
        let image_url = input_image(request);
        let mut model_id = model_id;
        let body = if model_id.contains("video") {
            if image_url.is_some() {
                // A text-to-video endpoint ignores `image_url`: start from the
                // image on the model's image-to-video sibling.
                model_id = image_to_video_endpoint(&model_id);
            }
            match image_url {
                Some(image_url) => serde_json::to_value(FalAIImageToVideoRequest {
                    image_url,
                    prompt,
                    duration,
                })?,
                None => serde_json::to_value(FalAITextToVideoRequest { prompt, duration })?,
            }
        } else {
            let image_size = request
                .params
                .as_ref()
                .and_then(|p| p.get("size"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(fal_image_size);
            serde_json::to_value(FalAITextToImageRequest {
                prompt,
                image_size,
                num_images: Some(1),
                image_url,
            })?
        };

        let submit = self.submit_direct(&model_id, &body).await?;
        Ok(SubmitOutcome::Async(JobHandle {
            provider_request_id: submit.request_id,
            poll_url: Some(submit.status_url),
            response_url: Some(submit.response_url),
        }))
    }

    async fn cancel(&self, handle: &JobHandle) -> KalpaResult<()> {
        let response_url = handle
            .response_url
            .as_deref()
            .ok_or_else(|| KalpaError::Config("Fal handle missing result URL".into()))?;
        let (model, request) = Self::parse_response_url(response_url)?;
        let url = format!("https://queue.fal.run/{model}/requests/{request}/cancel");
        crate::http::check_response(self.http_client.put(url).send().await?, "fal cancel").await?;
        Ok(())
    }

    async fn poll(&self, handle: &JobHandle) -> KalpaResult<PollStatus> {
        let poll_url = handle
            .poll_url
            .as_deref()
            .ok_or_else(|| KalpaError::ProviderError {
                status: 500,
                message: "fal job handle missing poll_url".into(),
            })?;

        match self.queue_status_by_url(poll_url).await? {
            FalQueueStatus::InQueue { queue_position, .. } => Ok(PollStatus::InQueue {
                position: queue_position,
            }),
            FalQueueStatus::InProgress { .. } => Ok(PollStatus::InProgress),
            FalQueueStatus::Failed { error, .. } => Ok(PollStatus::Failed(error)),
            FalQueueStatus::Completed { .. } => {
                let response_url =
                    handle
                        .response_url
                        .as_deref()
                        .ok_or_else(|| KalpaError::ProviderError {
                            status: 500,
                            message: "fal job handle missing response_url".into(),
                        })?;
                let result_json = self.queue_result_by_url(response_url).await?;
                // Parse generically: image responses carry `images[]`; video
                // responses carry `video` or `videos[]`.
                let mut parts = Vec::new();
                if let Ok(images) =
                    serde_json::from_value::<FalAIImageResponse>(result_json.clone())
                {
                    parts.extend(images.images.into_iter().map(|img| Part::Image {
                        url: Some(img.url),
                        b64_data: None,
                        mime: None,
                    }));
                }
                if parts.is_empty() {
                    if let Ok(video) = serde_json::from_value::<FalAIVideoResponse>(result_json) {
                        if let Some(v) = video.video {
                            parts.push(Part::Video {
                                url: v.url,
                                mime: Some("video/mp4".into()),
                            });
                        }
                        parts.extend(video.videos.into_iter().map(|v| Part::Video {
                            url: v.url,
                            mime: Some("video/mp4".into()),
                        }));
                    }
                }
                Ok(PollStatus::Completed(GenerationResponse {
                    model: handle.provider_request_id.clone(),
                    parts,
                    usage: None,
                }))
            }
        }
    }
}

fn validate_model_path(model: &str) -> KalpaResult<()> {
    if model.split('/').any(|part| {
        part.is_empty()
            || part == "."
            || part == ".."
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    }) {
        return Err(KalpaError::Config("Invalid Fal model path".into()));
    }
    Ok(())
}

#[cfg(test)]
mod security_tests {
    use super::*;
    #[test]
    fn rejects_untrusted_canonical_queue_urls() {
        for url in [
            "https://evil.example/model/requests/id/status",
            "http://queue.fal.run/model/requests/id",
            "https://queue.fal.run:444/model/requests/id",
            "https://user@queue.fal.run/model/requests/id",
        ] {
            assert!(FalAIProvider::trusted_queue_url(url).is_err());
            assert!(FalAIProvider::parse_response_url(url).is_err());
        }
        assert_eq!(
            FalAIProvider::parse_response_url(
                "https://queue.fal.run/fal-ai/flux/dev/requests/abc-123?logs=1"
            )
            .unwrap(),
            ("fal-ai/flux/dev".into(), "abc-123".into())
        );
    }
    #[test]
    fn rejects_model_path_injection() {
        for model in [
            "../model",
            "model?key=secret",
            "model#fragment",
            "model/%2f",
            "model//other",
        ] {
            assert!(validate_model_path(model).is_err());
        }
        assert!(validate_model_path("fal-ai/kling-video/v1.5/standard/text-to-video").is_ok());
    }

    #[test]
    fn an_image_selects_the_image_to_video_sibling() {
        assert_eq!(
            image_to_video_endpoint("fal-ai/minimax/video-01"),
            "fal-ai/minimax/video-01/image-to-video"
        );
        assert_eq!(
            image_to_video_endpoint("fal-ai/kling-video/v1.5/standard/text-to-video"),
            "fal-ai/kling-video/v1.5/standard/image-to-video"
        );
        assert_eq!(
            image_to_video_endpoint("fal-ai/minimax/video-01/image-to-video"),
            "fal-ai/minimax/video-01/image-to-video"
        );
    }

    #[test]
    fn openai_sizes_become_fal_dimensions_and_presets_pass() {
        assert_eq!(
            fal_image_size("1024x768"),
            serde_json::json!({"width": 1024, "height": 768})
        );
        assert_eq!(fal_image_size("square_hd"), serde_json::json!("square_hd"));
    }

    #[test]
    fn an_inline_input_image_is_sent_as_a_data_uri_not_dropped() {
        let mut request = GenerationRequest::prompt("fal-ai/minimax/video-01", "go");
        request.parts.push(Part::Image {
            url: None,
            b64_data: Some("AAAA".into()),
            mime: Some("image/jpeg".into()),
        });
        assert_eq!(
            input_image(&request).as_deref(),
            Some("data:image/jpeg;base64,AAAA")
        );
        let mut by_url = GenerationRequest::prompt("m", "go");
        by_url
            .parts
            .push(Part::image_url("https://example.com/a.png"));
        assert_eq!(
            input_image(&by_url).as_deref(),
            Some("https://example.com/a.png")
        );
        assert_eq!(input_image(&GenerationRequest::prompt("m", "go")), None);
    }
}
