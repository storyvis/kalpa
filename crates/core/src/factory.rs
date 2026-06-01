//! Provider factory — centralized provider construction from config.
//!
//! Eliminates repeated provider initialization code across CLI commands.

use std::path::Path;

use crate::auth::VertexAuthToken;
use crate::config::{KalpaConfig, Provider};
use crate::error::{KalpaError, KalpaResult};
use crate::providers::{ClaudeProvider, FalAIProvider, OpenAIProvider, VertexProvider};

/// Factory for creating provider instances from configuration.
///
/// Centralizes authentication and construction logic so callers
/// don't need to repeat service account loading, OAuth flows, etc.
pub struct ProviderFactory;

impl ProviderFactory {
    /// Create an OpenAI provider from config.
    pub fn openai(config: &KalpaConfig) -> KalpaResult<OpenAIProvider> {
        let api_key = config
            .get_api_key(Provider::OpenAI)
            .ok_or_else(|| KalpaError::Auth(
                "No API key configured for OpenAI. Run: kalpa configure --set openai.api_key YOUR_KEY".into()
            ))?
            .to_string();

        Ok(OpenAIProvider::new(api_key))
    }

    /// Create a Claude provider from config.
    pub fn claude(config: &KalpaConfig) -> KalpaResult<ClaudeProvider> {
        let api_key = config
            .get_api_key(Provider::Claude)
            .ok_or_else(|| KalpaError::Auth(
                "No API key configured for Claude. Run: kalpa configure --set claude.api_key YOUR_KEY".into()
            ))?
            .to_string();

        Ok(ClaudeProvider::new(api_key))
    }

    /// Create a Fal.ai provider from config.
    pub fn fal(config: &KalpaConfig) -> KalpaResult<FalAIProvider> {
        let api_key = config
            .get_api_key(Provider::Fal)
            .ok_or_else(|| KalpaError::Auth(
                "No API key configured for Fal.ai. Run: kalpa configure --set fal.api_key YOUR_KEY".into()
            ))?
            .to_string();

        Ok(FalAIProvider::new(api_key))
    }

    /// Create a Vertex AI provider from config (requires async for OAuth).
    pub async fn vertex(config: &KalpaConfig) -> KalpaResult<VertexProvider> {
        let service_account_path = config
            .get_service_account_path(Provider::Vertex)
            .ok_or_else(|| KalpaError::Auth(
                "No service account JSON configured for Vertex AI. Run: kalpa configure".into()
            ))?;

        let location = config
            .get_location(Provider::Vertex)
            .unwrap_or("us-central1")
            .to_string();

        let auth_token = VertexAuthToken::from_service_account_file(Path::new(service_account_path))
            .await
            .map_err(|e| KalpaError::Auth(format!("Failed to authenticate with Vertex AI: {}", e)))?;

        let project_id = auth_token.project_id.clone();
        let gcs_bucket = config.get_gcs_bucket(Provider::Vertex).map(|s| s.to_string());

        Ok(VertexProvider::new(auth_token.access_token, project_id, location, gcs_bucket))
    }

    /// Get the API key for a provider (non-Vertex).
    pub fn api_key(config: &KalpaConfig, provider: Provider) -> KalpaResult<String> {
        config
            .get_api_key(provider)
            .ok_or_else(|| KalpaError::Auth(format!(
                "No API key configured for {}. Run: kalpa configure --set {}.api_key YOUR_KEY",
                provider.display_name(),
                provider.as_str()
            )))
            .map(|s| s.to_string())
    }
}
