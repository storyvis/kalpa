//! Claude (Anthropic) provider implementation.
//!
//! Uses progenitor-generated client from kalpa-libgen.

use async_trait::async_trait;
use kalpa_libgen::claude;

use crate::error::{KalpaError, KalpaResult};
use crate::provider::CompletionProvider;
use crate::types::{CompletionRequest, CompletionResponse, Message, Role, Usage};

/// Claude provider for Anthropic's Claude models.
pub struct ClaudeProvider {
    client: claude::Client,
}

impl ClaudeProvider {
    /// Create a new Claude provider.
    ///
    /// # Arguments
    /// * `api_key` - Anthropic API key
    ///
    /// # Errors
    /// Returns an error if the API key contains invalid header characters.
    pub fn new(api_key: String) -> KalpaResult<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-api-key",
            reqwest::header::HeaderValue::from_str(&api_key)
                .map_err(|e| KalpaError::Auth(format!("Invalid API key format: {}", e)))?,
        );
        headers.insert(
            "anthropic-version",
            reqwest::header::HeaderValue::from_static("2023-06-01"),
        );
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        let http_client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|e| KalpaError::Config(format!("Failed to build HTTP client: {}", e)))?;

        let client = claude::Client::new_with_client("https://api.anthropic.com", http_client);

        Ok(Self { client })
    }

    /// Convert kalpa Messages to Claude InputMessages, separating system prompt.
    fn prepare_messages(
        messages: &[Message],
    ) -> (Option<String>, Vec<claude::types::InputMessage>) {
        let mut system_prompt: Option<String> = None;
        let mut conversation_messages = Vec::new();

        for message in messages {
            match message.role {
                Role::System => {
                    // Combine system messages if there are multiple
                    if let Some(existing) = system_prompt {
                        system_prompt = Some(format!("{}\n\n{}", existing, message.content));
                    } else {
                        system_prompt = Some(message.content.clone());
                    }
                }
                Role::User => {
                    conversation_messages.push(claude::types::InputMessage {
                        role: claude::types::InputMessageRole::User,
                        content: message.content.clone(),
                    });
                }
                Role::Assistant => {
                    conversation_messages.push(claude::types::InputMessage {
                        role: claude::types::InputMessageRole::Assistant,
                        content: message.content.clone(),
                    });
                }
            }
        }

        (system_prompt, conversation_messages)
    }
}

#[async_trait]
impl CompletionProvider for ClaudeProvider {
    fn name(&self) -> &str {
        "claude"
    }

    fn supported_models(&self) -> &[&str] {
        &[
            // Claude 4 series
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-sonnet-4-6",
            "claude-haiku-4-5-20251001",
            // Legacy naming for compatibility
            "claude-3-opus",
            "claude-3-sonnet",
            "claude-3-haiku",
        ]
    }

    async fn complete(&self, request: &CompletionRequest) -> KalpaResult<CompletionResponse> {
        // Separate system messages from user/assistant messages
        let (system_prompt, conversation_messages) = Self::prepare_messages(&request.messages);

        // Claude requires at least one message
        if conversation_messages.is_empty() {
            return Err(KalpaError::ProviderError {
                status: 400,
                message: "At least one user or assistant message is required".to_string(),
            });
        }

        let max_tokens = request.max_tokens.unwrap_or(4096) as i64;

        // Build Claude request
        let claude_request = claude::types::CreateMessageRequest {
            model: request.model.clone(),
            max_tokens,
            messages: conversation_messages,
            system: system_prompt,
            temperature: request.temperature.map(|t| t as f64),
            top_p: request.top_p.map(|t| t as f64),
            top_k: None,
            stop_sequences: request.stop_sequences.clone(),
        };

        // Make the API call
        let response = self
            .client
            .create_message(&claude_request)
            .await
            .map_err(|e| KalpaError::ProviderError {
                status: 500,
                message: format!("Claude API error: {}", e),
            })?;

        // Extract the response content
        let content = response
            .content
            .iter()
            .find_map(|block| block.text.clone())
            .ok_or_else(|| KalpaError::ProviderError {
                status: 500,
                message: "No text content in response".to_string(),
            })?;

        let usage = response.usage.as_ref().map(|u| Usage {
            prompt_tokens: u.input_tokens.unwrap_or(0) as u32,
            completion_tokens: u.output_tokens.unwrap_or(0) as u32,
            total_tokens: (u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0)) as u32,
        });

        Ok(CompletionResponse {
            content,
            model: response.model.clone().unwrap_or_else(|| request.model.clone()),
            usage,
            finish_reason: response.stop_reason.clone(),
        })
    }

    async fn health_check(&self) -> KalpaResult<()> {
        // Simple health check with minimal tokens
        let request = CompletionRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "Hi".to_string(),
            }],
            max_tokens: Some(10),
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };

        self.complete(&request).await?;
        Ok(())
    }
}
