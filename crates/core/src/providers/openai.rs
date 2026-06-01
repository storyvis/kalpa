//! OpenAI provider implementation.

use async_trait::async_trait;
use kalpa_libgen::openai;
use crate::error::{KalpaError, KalpaResult};
use crate::provider::{CompletionProvider, ImageGenerationProvider};
use crate::types::{
    CompletionRequest, CompletionResponse, ImageGenerationRequest, ImageGenerationResponse,
    GeneratedImage, Message, Role, Usage,
};

/// OpenAI provider for GPT models, DALL-E, and Sora.
pub struct OpenAIProvider {
    client: openai::Client,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider.
    ///
    /// # Arguments
    /// * `api_key` - OpenAI API key
    ///
    /// # Errors
    /// Returns an error if the API key contains invalid header characters.
    pub fn new(api_key: String) -> KalpaResult<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key))
                .map_err(|e| KalpaError::Auth(format!("Invalid API key format: {}", e)))?,
        );

        let http_client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|e| KalpaError::Config(format!("Failed to build HTTP client: {}", e)))?;

        let client = openai::Client::new_with_client("https://api.openai.com", http_client);

        Ok(Self { client })
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
