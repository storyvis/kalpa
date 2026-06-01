//! Configuration management for kalpa.
//!
//! Stores API keys and provider settings in `~/.config/kalpa/config.toml`.
//! The config file is created automatically on first use.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{KalpaError, KalpaResult};

/// The main configuration structure stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KalpaConfig {
    /// Provider-specific configurations keyed by provider name.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,

    /// Global settings.
    #[serde(default)]
    pub defaults: Defaults,
}

/// Per-provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Path to service account JSON file (for Vertex AI)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_account_path: Option<String>,

    /// GCS bucket URL for outputs (for Vertex AI)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gcs_bucket: Option<String>,

    /// GCS region/location (for Vertex AI)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// Global default settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Defaults {
    /// The default provider to use when none is specified.
    pub provider: String,

    /// Default output format.
    pub format: String,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            provider: "gemini".to_string(),
            format: "text".to_string(),
        }
    }
}

/// All supported provider identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Gemini,
    Vertex,
    Fal,
    OpenAI,
    Claude,
}

impl Provider {
    /// Get the string identifier for this provider.
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Gemini => "gemini",
            Provider::Vertex => "vertex",
            Provider::Fal => "fal",
            Provider::OpenAI => "openai",
            Provider::Claude => "claude",
        }
    }

    /// Parse a provider from string.
    pub fn from_str(s: &str) -> KalpaResult<Self> {
        match s.to_lowercase().as_str() {
            "gemini" | "g" => Ok(Provider::Gemini),
            "vertex" | "v" => Ok(Provider::Vertex),
            "fal" | "f" => Ok(Provider::Fal),
            "openai" | "o" => Ok(Provider::OpenAI),
            "claude" | "c" => Ok(Provider::Claude),
            _ => Err(KalpaError::Config(format!(
                "Unknown provider '{}'. Supported: gemini (g), vertex (v), fal (f), openai (o), claude (c)",
                s
            ))),
        }
    }

    /// Get all supported providers.
    pub fn all() -> &'static [Provider] {
        &[Provider::Gemini, Provider::Vertex, Provider::Fal, Provider::OpenAI, Provider::Claude]
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Provider::Gemini => "Google Gemini",
            Provider::Vertex => "Google Vertex AI",
            Provider::Fal => "Fal.ai",
            Provider::OpenAI => "OpenAI",
            Provider::Claude => "Claude (Anthropic)",
        }
    }
}

impl KalpaConfig {
    /// Get the config directory path (~/.config/kalpa/).
    pub fn config_dir() -> KalpaResult<PathBuf> {
        dirs::config_dir()
            .map(|d| d.join("kalpa"))
            .ok_or_else(|| KalpaError::Config("Could not determine config directory".into()))
    }

    /// Get the config file path (~/.config/kalpa/config.toml).
    pub fn config_path() -> KalpaResult<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    /// Load config from disk. Returns default config if file doesn't exist.
    pub fn load() -> KalpaResult<Self> {
        let path = Self::config_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path).map_err(|e| {
            KalpaError::Config(format!("Failed to read config at {}: {}", path.display(), e))
        })?;

        toml::from_str(&content).map_err(|e| {
            KalpaError::Config(format!("Failed to parse config: {}", e))
        })
    }

    /// Save config to disk.
    pub fn save(&self) -> KalpaResult<()> {
        let dir = Self::config_dir()?;
        fs::create_dir_all(&dir).map_err(|e| {
            KalpaError::Config(format!("Failed to create config dir {}: {}", dir.display(), e))
        })?;

        let path = Self::config_path()?;
        let content = toml::to_string_pretty(self).map_err(|e| {
            KalpaError::Config(format!("Failed to serialize config: {}", e))
        })?;

        fs::write(&path, &content).map_err(|e| {
            KalpaError::Config(format!("Failed to write config to {}: {}", path.display(), e))
        })?;

        Ok(())
    }

    /// Set the API key for a given provider.
    pub fn set_api_key(&mut self, provider: Provider, key: String) {
        let entry = self
            .providers
            .entry(provider.as_str().to_string())
            .or_default();
        entry.api_key = Some(key);
    }

    /// Get the API key for a given provider.
    pub fn get_api_key(&self, provider: Provider) -> Option<&str> {
        self.providers
            .get(provider.as_str())
            .and_then(|c| c.api_key.as_deref())
    }

    /// Set the service account path for Vertex AI.
    pub fn set_service_account_path(&mut self, provider: Provider, path: String) {
        let entry = self
            .providers
            .entry(provider.as_str().to_string())
            .or_default();
        entry.service_account_path = Some(path);
    }

    /// Get the service account path for a provider (mainly for Vertex AI).
    pub fn get_service_account_path(&self, provider: Provider) -> Option<&str> {
        self.providers
            .get(provider.as_str())
            .and_then(|c| c.service_account_path.as_deref())
    }

    /// Set the default model for a provider.
    pub fn set_default_model(&mut self, provider: Provider, model: String) {
        let entry = self
            .providers
            .entry(provider.as_str().to_string())
            .or_default();
        entry.default_model = Some(model);
    }

    /// Get the default model for a provider.
    pub fn get_default_model(&self, provider: Provider) -> Option<&str> {
        self.providers
            .get(provider.as_str())
            .and_then(|c| c.default_model.as_deref())
    }

    /// Set the GCS bucket for Vertex AI.
    pub fn set_gcs_bucket(&mut self, provider: Provider, bucket: String) {
        let entry = self
            .providers
            .entry(provider.as_str().to_string())
            .or_default();
        entry.gcs_bucket = Some(bucket);
    }

    /// Get the GCS bucket for a provider (mainly for Vertex AI).
    pub fn get_gcs_bucket(&self, provider: Provider) -> Option<&str> {
        self.providers
            .get(provider.as_str())
            .and_then(|c| c.gcs_bucket.as_deref())
    }

    /// Set the location/region for Vertex AI.
    pub fn set_location(&mut self, provider: Provider, location: String) {
        let entry = self
            .providers
            .entry(provider.as_str().to_string())
            .or_default();
        entry.location = Some(location);
    }

    /// Get the location/region for a provider (mainly for Vertex AI).
    pub fn get_location(&self, provider: Provider) -> Option<&str> {
        self.providers
            .get(provider.as_str())
            .and_then(|c| c.location.as_deref())
    }

    /// Check if a provider is configured (has an API key or service account).
    pub fn is_configured(&self, provider: Provider) -> bool {
        match provider {
            Provider::Vertex => self.get_service_account_path(provider).is_some(),
            _ => self.get_api_key(provider).is_some(),
        }
    }
}
