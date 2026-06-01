//! Vertex AI provider implementation.

use async_trait::async_trait;
use kalpa_libgen::vertex;
use tracing::{info, instrument};
use uuid::Uuid;

use crate::error::{KalpaError, KalpaResult};
use crate::provider::{CompletionProvider, ImageGenerationProvider, VideoGenerationProvider};
use crate::types::{
    CompletionRequest, CompletionResponse, ImageGenerationRequest, ImageGenerationResponse,
    GeneratedImage, GeneratedVideo, Message, Role, Usage, VideoGenerationRequest,
    VideoGenerationResponse,
};
use std::time::Duration;
use tokio::time::sleep;

/// Vertex AI provider for Gemini models, Imagen, and Veo.
pub struct VertexProvider {
    client: vertex::Client,
    /// Shared HTTP client for direct API calls (GCS, polling).
    http_client: reqwest::Client,
    project_id: String,
    location: String,
    bearer_token: String,
    gcs_bucket: Option<String>,
}

impl VertexProvider {
    /// Create a new Vertex AI provider.
    ///
    /// # Arguments
    /// * `bearer_token` - OAuth2/JWT token for authentication
    /// * `project_id` - Google Cloud project ID
    /// * `location` - Cloud region (e.g., "us-central1")
    /// * `gcs_bucket` - Optional GCS bucket for outputs (e.g., "gs://my-bucket")
    ///
    /// # Errors
    /// Returns an error if the bearer token contains invalid header characters.
    pub fn new(bearer_token: String, project_id: String, location: String, gcs_bucket: Option<String>) -> KalpaResult<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", bearer_token))
                .map_err(|e| KalpaError::Auth(format!("Invalid bearer token format: {}", e)))?,
        );

        let http_client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|e| KalpaError::Config(format!("Failed to build HTTP client: {}", e)))?;

        let base_url = format!("https://{}-aiplatform.googleapis.com", location);
        let client = vertex::Client::new_with_client(&base_url, http_client.clone());

        Ok(Self {
            client,
            http_client,
            project_id,
            location,
            bearer_token,
            gcs_bucket,
        })
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
            parts: Some(parts),
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
            .as_ref()?
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
            "gemini-3.1-flash",           // Nano Banana 2
            "gemini-3-pro",               // Nano Banana Pro
            "gemini-2.5-flash",           // Original Nano Banana
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
        // Convert messages to Vertex format
        let contents: Vec<_> = request
            .messages
            .iter()
            .map(Self::convert_message_to_content)
            .collect();

        // Build generation config
        let generation_config = Some(vertex::types::GenerationConfig {
            temperature: request.temperature,
            max_output_tokens: request.max_tokens.map(|t| t as i64),
            top_p: request.top_p,
            top_k: None,
            stop_sequences: request.stop_sequences.clone(),
            response_mime_type: None,
        });

        let vertex_request = vertex::types::GenerateRequest {
            contents,
            generation_config,
            safety_settings: None,
        };

        // Make the API call
        let response = self
            .client
            .generate_content(
                &self.project_id,
                &self.location,
                &request.model,
                &vertex_request,
            )
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("Vertex AI error: {}", e),
            })?;

        // Extract the response
        let candidate = response
            .candidates
            .as_ref()
            .and_then(|c| c.first())
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
            finish_reason: candidate.finish_reason.map(|r| format!("{:?}", r)),
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
        // Build Vertex AI request using :predict format for Imagen
        let instances = vec![vertex::types::ImageInstance {
            prompt: request.prompt.clone(),
        }];

        let parameters = Some(vertex::types::ImageParameters {
            sample_count: Some(1),
            aspect_ratio: None,
            negative_prompt: None,
        });

        let predict_request = vertex::types::PredictRequest {
            instances,
            parameters,
        };

        // Make the API call using :predict endpoint (required for Imagen models)
        let response = self
            .client
            .predict(
                &self.project_id,
                &self.location,
                &request.model,
                &predict_request,
            )
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("Vertex AI image generation error: {}", e),
            })?;

        // Extract image data from response
        let images: Vec<GeneratedImage> = if let Some(predictions) = &response.predictions {
            predictions
                .iter()
                .filter_map(|prediction| {
                    prediction.bytes_base64_encoded.as_ref().map(|data| GeneratedImage {
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
        let bucket_name = if let Some(gcs_uri) = &self.gcs_bucket {
            // Parse "gs://bucket-name/path" to get "bucket-name"
            gcs_uri.strip_prefix("gs://")
                .and_then(|s| s.split('/').next())
                .ok_or_else(|| KalpaError::ProviderError {
                    status: 500,
                    message: format!("Invalid GCS bucket URI format: {}", gcs_uri),
                })?
        } else {
            // Fallback bucket name
            &format!("{}-kalpa-videos", self.project_id)
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
        let response = self
            .client
            .predict_long_running(
                &self.project_id,
                &self.location,
                &request.model,
                &video_request,
            )
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("Failed to start video generation: {}", e),
            })?;

        // Extract operation response
        let operation_response = response.into_inner();

        // Check if this is a long-running operation
        if let Some(op_name) = &operation_response.name {
            // Poll for operation completion
            let final_response = self
                .poll_operation(op_name)
                .await?;

            // STEP 3: Extract video data from completed operation using request_id
            let videos = self.extract_videos_from_operation(&final_response, &request_id, bucket_name).await?;

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

        let response = self.http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("Failed to list GCS objects: {}", e),
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(KalpaError::ProviderError {
                status: status.as_u16(),
                message: format!("GCS listing failed: {} - {}", status, error_text),
            });
        }

        let response_text = response.text().await.map_err(|e| KalpaError::ProviderError {
            status: 500,
            message: format!("Failed to read GCS response: {}", e),
        })?;

        let gcs_response: GcsListResponse = serde_json::from_str(&response_text)
            .map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("Failed to parse GCS response: {}", e),
            })?;

        Ok(gcs_response.items.unwrap_or_default())
    }

    /// Poll a long-running operation until completion
    async fn poll_operation(
        &self,
        operation_name: &str,
    ) -> KalpaResult<vertex::types::OperationResponse> {
        let max_attempts = 120; // Max 10 minutes (5 seconds * 120)
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
                "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:fetchPredictOperation",
                self.location, self.project_id, self.location, model_name
            );
            
            // The request body must contain only the operation name
            // The GCS URI is already in the response since we passed storageUri in the initial request
            let body = serde_json::json!({
                "operationName": operation_name
            });
            
            let http_response = self.http_client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| KalpaError::ProviderError {
                    status: 500,
                    message: format!("Failed to send poll request: {}", e),
                })?;

            let status = http_response.status();
            if !status.is_success() {
                let error_text = http_response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                return Err(KalpaError::ProviderError {
                    status: status.as_u16(),
                    message: format!("Failed to poll operation: {} - {}", status, error_text),
                });
            }

            let response_text = http_response.text().await.map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("Failed to read response text: {}", e),
            })?;

            let operation_response: vertex::types::OperationResponse = serde_json::from_str(&response_text)
                .map_err(|e| KalpaError::ProviderError {
                    status: 500,
                    message: format!("Failed to parse operation response: {}", e),
                })?;

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
        let response_json = serde_json::to_value(response).map_err(|e| KalpaError::ProviderError {
            status: 500,
            message: format!("Failed to serialize response: {}", e),
        })?;
        
        // Try to extract GCS URIs from response.generateVideoResponse.generatedSamples[].video.uri
        if let Some(response_obj) = response_json.get("response") {
            // Check for generateVideoResponse (Veo 2.0 format)
            if let Some(video_response) = response_obj.get("generateVideoResponse") {
                if let Some(samples) = video_response.get("generatedSamples").and_then(|s| s.as_array()) {
                    let videos: Vec<GeneratedVideo> = samples
                        .iter()
                        .filter_map(|sample| {
                            sample.get("video")
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
            if let Some(predict_response) = &response.response {
                if let Some(predictions) = &predict_response.predictions {
                    let videos: Vec<GeneratedVideo> = predictions
                        .iter()
                        .filter_map(|prediction| {
                            prediction.bytes_base64_encoded.as_ref().map(|data| {
                                GeneratedVideo {
                                    url: format!("data:video/mp4;base64,{}", data),
                                }
                            })
                        })
                        .collect();

                    if !videos.is_empty() {
                        return Ok(videos);
                    }
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
        
        Ok(vec![GeneratedVideo {
            url: video_uri,
        }])
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
