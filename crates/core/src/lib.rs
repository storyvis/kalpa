//! # kalpa-core
//!
//! Core traits, types, and configuration for AI provider integrations.
//! All AI providers (Gemini, Vertex, Fal, OpenAI, etc.) implement these traits
//! to ensure a consistent, production-grade interface.

pub mod auth;
pub mod config;
pub mod dispatcher;
pub mod error;
pub mod factory;
pub mod generation;
pub mod http;
pub mod jobs;
pub mod provider;
pub mod providers;
pub mod ratelimit;
pub mod registry;
pub mod retry;
pub mod types;

pub use config::{KalpaConfig, Provider, ProviderConfig};
pub use error::{KalpaError, KalpaResult};
pub use generation::{
    GenerationRequest, GenerationResponse, Modality, ModelRef, Part, SpeechRequest,
    TranscriptionRequest,
};
pub use provider::{
    CompletionProvider, EmbeddingProvider, GenerationProvider, ImageGenerationProvider, JobHandle,
    PollStatus, SpeechProvider, SubmitOutcome, TranscriptionProvider, VideoGenerationProvider,
};
pub use dispatcher::Dispatcher;
pub use ratelimit::{AimdConfig, AimdLimiter, BindingSpec, LimiterRegistry, Permit};
pub use registry::{Binding, ModelInfo, Registry, Resolved};
pub use types::*;
