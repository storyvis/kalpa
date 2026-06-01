//! Fal AI provider implementation.
//!
//! Supports various Fal AI models for text-to-image, image-to-video, and text-to-video generation.
//! Video generation can use direct API or queue-based API depending on the model.

use async_trait::async_trait;
use kalpa_libgen::falai;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::{KalpaError, KalpaResult};
use crate::provider::{ImageGenerationProvider, VideoGenerationProvider};
use crate::types::{
    GeneratedImage, GeneratedVideo, ImageGenerationRequest, ImageGenerationResponse,
    VideoGenerationRequest, VideoGenerationResponse, FalQueueSubmitResponse, FalQueueStatus,
};

/// Fal AI provider for various generative models.
pub struct FalAIProvider {
    client: falai::Client,
    http_client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct FalAITextToImageRequest {
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_images: Option<i32>,
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
    ///
    /// # Errors
    /// Returns an error if the API key contains invalid header characters.
    pub fn new(api_key: String) -> KalpaResult<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Key {}", api_key))
                .map_err(|e| KalpaError::Auth(format!("Invalid API key format: {}", e)))?,
        );

        let reqwest_client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(300)) // 5 minute timeout for individual requests
            .build()
            .map_err(|e| KalpaError::Config(format!("Failed to build HTTP client: {}", e)))?;

        let client = falai::Client::new_with_client(
            "https://queue.fal.run",
            reqwest_client.clone(),
        );

        Ok(Self { client, http_client: reqwest_client })
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
        let file_data = tokio::fs::read(file_path)
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 400u16,
                message: format!("Failed to read file: {}", e),
            })?;

        // Detect content type from extension
        let content_type = match Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
        {
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
    pub async fn queue_submit<T>(&self, model_id: &str, request: &T) -> KalpaResult<FalQueueSubmitResponse>
    where
        T: Serialize,
    {
        // Serialize request to serde_json::Value and convert to Map
        let request_value = serde_json::to_value(request)
            .map_err(|e| KalpaError::ProviderError {
                status: 400u16,
                message: format!("Failed to serialize request: {}", e),
            })?;
        
        let request_body = request_value.as_object()
            .ok_or_else(|| KalpaError::ProviderError {
                status: 400u16,
                message: "Request must be a JSON object".to_string(),
            })?;

        // Use progenitor-generated method
        let response = self
            .client
            .submit_to_queue(model_id, request_body)
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500u16,
                message: format!("Fal AI queue submit failed: {}", e),
            })?
            .into_inner();

        // Convert from generated type to our custom type
        Ok(FalQueueSubmitResponse {
            request_id: response.request_id,
            status_url: response.status_url,
            response_url: response.response_url,
            cancel_url: response.cancel_url,
        })
    }

    /// Check status of a queued request using progenitor-generated client
    pub async fn queue_status(&self, model_id: &str, request_id: &str) -> KalpaResult<FalQueueStatus> {
        // Use progenitor-generated method
        let response = self
            .client
            .get_queue_status(model_id, request_id, Some("1"))
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500u16,
                message: format!("Fal AI queue status check failed: {}", e),
            })?
            .into_inner();

        // Convert from generated response to our custom enum via JSON serialization
        let status_json = serde_json::to_value(&response)
            .map_err(|e| KalpaError::ProviderError {
                status: 500u16,
                message: format!("Failed to serialize status response: {}", e),
            })?;
        
        let status: FalQueueStatus = serde_json::from_value(status_json)
            .map_err(|e| KalpaError::ProviderError {
                status: 500u16,
                message: format!("Failed to parse status response: {}", e),
            })?;

        Ok(status)
    }

    /// Check status using a direct status URL (preferred over queue_status).
    /// Fal.ai returns a canonical status_url in the submit response.
    /// Makes a direct GET request to the status URL with authentication.
    pub async fn queue_status_by_url(&self, status_url: &str) -> KalpaResult<FalQueueStatus> {
        // Build the URL with logs parameter
        let url = if status_url.contains('?') {
            format!("{}&logs=1", status_url)
        } else {
            format!("{}?logs=1", status_url)
        };

        // Make a direct GET request to the status URL
        let response = self.http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500u16,
                message: format!("Fal AI queue status request failed: {}", e),
            })?;

        if !response.status().is_success() {
            let status_code = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(KalpaError::ProviderError {
                status: status_code,
                message: format!("Fal AI queue status check failed (HTTP {}): {}", status_code, body),
            });
        }

        let status: FalQueueStatus = response.json().await
            .map_err(|e| KalpaError::ProviderError {
                status: 500u16,
                message: format!("Failed to parse status response: {}", e),
            })?;

        Ok(status)
    }

    /// Parse a fal.ai response URL to extract model_id and request_id.
    /// Expected format: https://queue.fal.run/{model_id}/requests/{request_id}
    fn parse_response_url(response_url: &str) -> KalpaResult<(String, String)> {
        // Strip query parameters if present
        let url_path = response_url.split('?').next().unwrap_or(response_url);

        // Remove the base URL prefix to get the path
        let path = if let Some(stripped) = url_path.strip_prefix("https://queue.fal.run/") {
            stripped
        } else if let Some(stripped) = url_path.strip_prefix("http://queue.fal.run/") {
            stripped
        } else {
            url_path
        };

        // Find the "/requests/" separator
        if let Some(requests_idx) = path.find("/requests/") {
            let model_id = &path[..requests_idx];
            let request_id = &path[requests_idx + "/requests/".len()..];

            Ok((model_id.to_string(), request_id.to_string()))
        } else {
            Err(KalpaError::ProviderError {
                status: 400u16,
                message: format!(
                    "Invalid fal.ai response URL format. Expected '.../{{model_id}}/requests/{{request_id}}', got: {}",
                    response_url
                ),
            })
        }
    }

    /// Get the result of a completed queue request.
    /// Returns the raw JSON value of the result.
    pub async fn queue_result(&self, model_id: &str, request_id: &str) -> KalpaResult<serde_json::Value> {
        let result = self
            .client
            .get_queue_result(model_id, request_id)
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500u16,
                message: format!("Failed to fetch result: {}", e),
            })?
            .into_inner();

        serde_json::to_value(&result)
            .map_err(|e| KalpaError::ProviderError {
                status: 500u16,
                message: format!("Failed to serialize result: {}", e),
            })
    }

    /// Get the result of a completed queue request using the canonical response URL.
    /// Parses the URL to extract model_id and request_id, then delegates to queue_result.
    pub async fn queue_result_by_url(&self, response_url: &str) -> KalpaResult<serde_json::Value> {
        let (model_id, request_id) = Self::parse_response_url(response_url)?;
        self.queue_result(&model_id, &request_id).await
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
    async fn queue_submit_and_wait<T, R>(
        &self,
        model_id: &str,
        request: &T,
    ) -> KalpaResult<R>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        // Submit to queue using progenitor-generated method
        let submit_response = self.queue_submit(model_id, request).await?;

        // Use the canonical URLs from the submit response for polling
        let status_url = submit_response.status_url.clone();
        let response_url = submit_response.response_url.clone();

        // Poll for completion using direct GET to the status_url
        loop {
            let status = self.queue_status_by_url(&status_url).await?;

            match status {
                FalQueueStatus::Completed { .. } => {
                    // Fetch the actual result using direct GET to the response_url
                    let result_json = self.queue_result_by_url(&response_url).await?;

                    return serde_json::from_value(result_json).map_err(|e| {
                        KalpaError::ProviderError {
                            status: 500u16,
                            message: format!("Failed to parse result: {}", e),
                        }
                    });
                }
                FalQueueStatus::Failed { error, .. } => {
                    return Err(KalpaError::ProviderError {
                        status: 500u16,
                        message: format!("Fal AI generation failed: {}", error),
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
        "falai"
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
            image_size: request.size.clone(),
            num_images: Some(1),
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
        "falai"
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
            let image_url = request
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
            self.queue_submit_and_wait(&request.model, &fal_request).await?
        } else {
            // Text-to-video
            let fal_request = FalAITextToVideoRequest {
                prompt: request.prompt.clone(),
                duration: request.duration,
            };

            // Use queue-based API for all video generations
            self.queue_submit_and_wait(&request.model, &fal_request).await?
        };

        // Handle both response formats: single video or videos array
        let videos = if let Some(video) = response.video {
            vec![GeneratedVideo { url: video.url }]
        } else if !response.videos.is_empty() {
            response.videos
                .into_iter()
                .map(|v| GeneratedVideo { url: v.url })
                .collect()
        } else {
            // Try to extract from raw response data for debugging
            let debug_msg = format!(
                "No video in response. Response structure: {:?}",
                serde_json::to_string(&response).unwrap_or_else(|_| "unable to serialize".to_string())
            );
            return Err(KalpaError::ProviderError {
                status: 500u16,
                message: debug_msg,
            });
        };

        Ok(VideoGenerationResponse {
            videos,
            model: request.model.clone(),
        })
    }
}
