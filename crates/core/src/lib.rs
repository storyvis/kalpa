//! # kalpa-core
//!
//! Core traits, types, and configuration for AI provider integrations.
//! All AI providers (Gemini, Vertex, Fal, OpenAI, etc.) implement these traits
//! to ensure a consistent, production-grade interface.

pub mod auth;
pub mod config;
pub mod error;
pub mod factory;
pub mod jobs;
pub mod provider;
pub mod providers;
pub mod registry;
pub mod retry;
pub mod types;

pub use config::{KalpaConfig, Provider, ProviderConfig};
pub use error::{KalpaError, KalpaResult};
pub use factory::ProviderFactory;
pub use provider::{CompletionProvider, EmbeddingProvider, ImageGenerationProvider, VideoGenerationProvider};
pub use registry::{ContentKind, ModelEntry};
pub use types::*;
