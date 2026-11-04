//! Generation dispatcher: resolve a model ref → AIMD-gate → drive the provider
//! lifecycle (sync inline, or async submit→poll) → return a unified response.
//!
//! M1 uses a single global limiter; M3 swaps in a per-binding `LimiterRegistry`
//! nested under a per-provider parent (the `generate` flow is unchanged).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::error::{KalpaError, KalpaResult};
use crate::generation::{
    GenerationRequest, GenerationResponse, ModelRef, SpeechRequest, TranscriptionRequest,
};
use crate::provider::{
    CancelOnDrop, GenerationProvider, PollStatus, SpeechProvider, SubmitOutcome,
    TranscriptionProvider,
};
use crate::ratelimit::LimiterRegistry;
use crate::registry::Registry;

/// Routes generation requests to providers, gated by per-binding AIMD limiters
/// (each nested under a per-provider parent ceiling).
pub struct Dispatcher {
    registry: Registry,
    providers: HashMap<String, Arc<dyn GenerationProvider>>,
    speech: HashMap<String, Arc<dyn SpeechProvider>>,
    transcription: HashMap<String, Arc<dyn TranscriptionProvider>>,
    limiters: Arc<LimiterRegistry>,
    poll_interval: Duration,
    generation_timeout: Duration,
}

impl Dispatcher {
    /// Build a dispatcher from a registry, provider instances (keyed by provider
    /// name), and the limiter registry.
    pub fn new(
        registry: Registry,
        providers: HashMap<String, Arc<dyn GenerationProvider>>,
        limiters: Arc<LimiterRegistry>,
    ) -> Self {
        Self {
            registry,
            providers,
            speech: HashMap::new(),
            transcription: HashMap::new(),
            limiters,
            poll_interval: Duration::from_secs(2),
            generation_timeout: Duration::from_secs(900),
        }
    }

    /// Register text-to-speech providers (keyed by provider name).
    pub fn with_speech_providers(
        mut self,
        speech: HashMap<String, Arc<dyn SpeechProvider>>,
    ) -> Self {
        self.speech = speech;
        self
    }

    /// Register transcription providers (keyed by provider name).
    pub fn with_transcription_providers(
        mut self,
        transcription: HashMap<String, Arc<dyn TranscriptionProvider>>,
    ) -> Self {
        self.transcription = transcription;
        self
    }

    /// Override the async poll interval.
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval.max(Duration::from_millis(10));
        self
    }

    /// Override the overall generation deadline (including admission and polling).
    pub fn with_generation_timeout(mut self, timeout: Duration) -> Self {
        self.generation_timeout = timeout;
        self
    }

    /// The model catalog backing this dispatcher (for `/v1/models`).
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Resolve, gate, and run a single generation to a terminal result.
    pub async fn generate(&self, request: &GenerationRequest) -> KalpaResult<GenerationResponse> {
        tokio::time::timeout(self.generation_timeout, self.generate_inner(request))
            .await
            .map_err(|_| KalpaError::ProviderError {
                status: 504,
                message: "Generation deadline exceeded".into(),
            })?
    }

    async fn generate_inner(&self, request: &GenerationRequest) -> KalpaResult<GenerationResponse> {
        let resolved = self.registry.resolve(&request.model)?;
        let provider = self
            .providers
            .get(&resolved.binding.provider)
            .ok_or_else(|| {
                KalpaError::Config(format!(
                    "No provider instance for '{}'",
                    resolved.binding.provider
                ))
            })?;

        // Pass the provider-specific slug down as the model id.
        let mut preq = request.clone();
        preq.model = ModelRef(resolved.binding.provider_slug.clone());

        // Both binding and provider account permits span the whole lifecycle;
        // completions and submit-time back-pressure adapt both ceilings.
        let (parent, binding) = self.limiters.limiter_for(
            &resolved.binding.provider,
            &resolved.binding.provider_slug,
            resolved.binding.region.as_deref(),
        );
        // Wait for binding capacity before taking shared account capacity, so
        // one busy binding cannot reserve every provider permit while queued.
        let permit = binding.acquire().await;
        let parent_permit = parent.acquire().await;
        let start = Instant::now();

        let outcome = match provider.submit(&preq).await {
            Ok(o) => o,
            Err(e) if capacity_error(&e) => {
                parent.on_submit_rejected(None);
                binding.on_submit_rejected(None); // fast-loop MD
                return Err(e);
            }
            Err(e) => return Err(e),
        };

        match outcome {
            SubmitOutcome::Sync(resp) => {
                parent.on_completed(&parent_permit, start.elapsed());
                binding.on_completed(&permit, start.elapsed());
                Ok(resp)
            }
            SubmitOutcome::Async(handle) => {
                let mut cancellation = CancelOnDrop {
                    provider: Arc::clone(provider),
                    handle: Some(handle.clone()),
                };
                let mut poll_backoff = self.poll_interval;
                loop {
                    let status = match provider.poll(&handle).await {
                        Ok(status) => {
                            poll_backoff = self.poll_interval;
                            status
                        }
                        Err(error) if capacity_error(&error) => {
                            tokio::time::sleep(poll_backoff).await;
                            poll_backoff =
                                poll_backoff.saturating_mul(2).min(Duration::from_secs(30));
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    match status {
                        PollStatus::InQueue { position } => {
                            binding.observe_queue(position, start.elapsed());
                            tokio::time::sleep(self.poll_interval).await;
                        }
                        PollStatus::InProgress => {
                            tokio::time::sleep(self.poll_interval).await;
                        }
                        PollStatus::Completed(resp) => {
                            cancellation.handle = None;
                            parent.on_completed(&parent_permit, start.elapsed());
                            binding.on_completed(&permit, start.elapsed());
                            return Ok(resp);
                        }
                        PollStatus::Failed(msg) => {
                            cancellation.handle = None;
                            binding.on_failed(false);
                            return Err(KalpaError::ProviderError {
                                status: 500,
                                message: msg,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Text-to-speech, AIMD-gated like `generate` (synchronous provider).
    pub async fn synthesize(&self, request: &SpeechRequest) -> KalpaResult<GenerationResponse> {
        let resolved = self.registry.resolve(&request.model)?;
        let provider = self.speech.get(&resolved.binding.provider).ok_or_else(|| {
            KalpaError::Config(format!(
                "No speech provider for '{}'",
                resolved.binding.provider
            ))
        })?;
        let mut req = request.clone();
        req.model = ModelRef(resolved.binding.provider_slug.clone());

        let (parent, binding) = self.limiters.limiter_for(
            &resolved.binding.provider,
            &resolved.binding.provider_slug,
            resolved.binding.region.as_deref(),
        );
        // Wait for binding capacity before taking shared account capacity, so
        // one busy binding cannot reserve every provider permit while queued.
        let permit = binding.acquire().await;
        let parent_permit = parent.acquire().await;
        let start = Instant::now();
        match provider.synthesize(&req).await {
            Ok(resp) => {
                parent.on_completed(&parent_permit, start.elapsed());
                binding.on_completed(&permit, start.elapsed());
                Ok(resp)
            }
            Err(e) if capacity_error(&e) => {
                parent.on_submit_rejected(None);
                binding.on_submit_rejected(None);
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    /// Speech-to-text transcription, AIMD-gated.
    pub async fn transcribe(
        &self,
        request: &TranscriptionRequest,
    ) -> KalpaResult<GenerationResponse> {
        let resolved = self.registry.resolve(&request.model)?;
        let provider = self
            .transcription
            .get(&resolved.binding.provider)
            .ok_or_else(|| {
                KalpaError::Config(format!(
                    "No transcription provider for '{}'",
                    resolved.binding.provider
                ))
            })?;
        let mut req = request.clone();
        req.model = ModelRef(resolved.binding.provider_slug.clone());

        let (parent, binding) = self.limiters.limiter_for(
            &resolved.binding.provider,
            &resolved.binding.provider_slug,
            resolved.binding.region.as_deref(),
        );
        // Wait for binding capacity before taking shared account capacity, so
        // one busy binding cannot reserve every provider permit while queued.
        let permit = binding.acquire().await;
        let parent_permit = parent.acquire().await;
        let start = Instant::now();
        match provider.transcribe(&req).await {
            Ok(resp) => {
                parent.on_completed(&parent_permit, start.elapsed());
                binding.on_completed(&permit, start.elapsed());
                Ok(resp)
            }
            Err(e) if capacity_error(&e) => {
                parent.on_submit_rejected(None);
                binding.on_submit_rejected(None);
                Err(e)
            }
            Err(e) => Err(e),
        }
    }
}

fn capacity_error(error: &KalpaError) -> bool {
    matches!(
        error,
        KalpaError::RateLimited(_)
            | KalpaError::ProviderError {
                status: 429 | 503,
                ..
            }
    ) || matches!(error, KalpaError::Http(error) if error.is_timeout())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::Part;
    use crate::provider::JobHandle;
    use crate::ratelimit::{AimdConfig, LimiterRegistry};
    use async_trait::async_trait;
    use std::collections::HashMap as Map;

    struct MockFal;

    #[async_trait]
    impl GenerationProvider for MockFal {
        fn name(&self) -> &str {
            "fal"
        }
        async fn submit(&self, req: &GenerationRequest) -> KalpaResult<SubmitOutcome> {
            // Echo the provider slug back so the test can assert the rewrite.
            Ok(SubmitOutcome::Sync(GenerationResponse {
                model: req.model.0.clone(),
                parts: vec![Part::image_url("https://example/out.png")],
                usage: None,
            }))
        }
        async fn poll(&self, _h: &JobHandle) -> KalpaResult<PollStatus> {
            unreachable!()
        }
    }

    struct MockTts;

    #[async_trait]
    impl SpeechProvider for MockTts {
        fn name(&self) -> &str {
            "openai"
        }
        async fn synthesize(&self, req: &SpeechRequest) -> KalpaResult<GenerationResponse> {
            Ok(GenerationResponse {
                model: req.model.0.clone(),
                parts: vec![Part::Audio {
                    url: None,
                    b64_data: Some("AAAA".into()),
                    mime: Some("audio/mpeg".into()),
                }],
                usage: None,
            })
        }
    }

    #[tokio::test]
    async fn synthesize_routes_and_gates() {
        let mut speech: Map<String, Arc<dyn SpeechProvider>> = Map::new();
        speech.insert("openai".into(), Arc::new(MockTts));
        let d = Dispatcher::new(
            Registry::with_defaults(),
            Map::new(),
            Arc::new(LimiterRegistry::new(AimdConfig::default(), Map::new())),
        )
        .with_speech_providers(speech);

        let req = SpeechRequest {
            model: "tts-1".into(),
            input: "hello".into(),
            voice: None,
            format: None,
        };
        let resp = d.synthesize(&req).await.unwrap();
        assert_eq!(resp.model, "tts-1"); // rewritten to provider_slug
        assert!(matches!(resp.parts[0], Part::Audio { .. }));
    }

    #[tokio::test]
    async fn resolves_gates_and_runs() {
        let mut providers: HashMap<String, Arc<dyn GenerationProvider>> = HashMap::new();
        providers.insert("fal".into(), Arc::new(MockFal));
        let d = Dispatcher::new(
            Registry::with_defaults(),
            providers,
            Arc::new(LimiterRegistry::new(AimdConfig::default(), Map::new())),
        );

        let resp = d
            .generate(&GenerationRequest::prompt("flux-dev", "a cat"))
            .await
            .unwrap();
        // The dispatcher rewrote the logical slug to the binding's provider_slug.
        assert_eq!(resp.model, "fal-ai/flux/dev");
        assert_eq!(resp.image_urls(), vec!["https://example/out.png"]);
    }
    struct QueuedProvider {
        polled: std::sync::atomic::AtomicUsize,
        complete: bool,
        cancelled: tokio::sync::Notify,
    }
    #[async_trait]
    impl GenerationProvider for QueuedProvider {
        fn name(&self) -> &str {
            "fal"
        }
        async fn submit(&self, _: &GenerationRequest) -> KalpaResult<SubmitOutcome> {
            Ok(SubmitOutcome::Async(JobHandle {
                provider_request_id: "test".into(),
                poll_url: None,
                response_url: None,
            }))
        }
        async fn poll(&self, _: &JobHandle) -> KalpaResult<PollStatus> {
            if self
                .polled
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                == 0
            {
                return Err(KalpaError::RateLimited("poll only".into()));
            }
            if self.complete {
                Ok(PollStatus::Completed(GenerationResponse {
                    model: "fal-ai/flux/dev".into(),
                    parts: Vec::new(),
                    usage: None,
                }))
            } else {
                Ok(PollStatus::InProgress)
            }
        }
        async fn cancel(&self, _: &JobHandle) -> KalpaResult<()> {
            self.cancelled.notify_one();
            Ok(())
        }
    }
    fn queued_dispatcher(
        complete: bool,
    ) -> (Dispatcher, Arc<QueuedProvider>, Arc<LimiterRegistry>) {
        let provider = Arc::new(QueuedProvider {
            polled: Default::default(),
            complete,
            cancelled: Default::default(),
        });
        let mut providers: HashMap<String, Arc<dyn GenerationProvider>> = HashMap::new();
        providers.insert("fal".into(), provider.clone());
        let limiters = Arc::new(LimiterRegistry::new(AimdConfig::default(), HashMap::new()));
        (
            Dispatcher::new(Registry::with_defaults(), providers, limiters.clone())
                .with_poll_interval(Duration::from_millis(10)),
            provider,
            limiters,
        )
    }
    #[tokio::test]
    async fn poll_rate_limit_retries_without_decreasing_admission() {
        let (dispatcher, provider, limiters) = queued_dispatcher(true);
        dispatcher
            .generate(&GenerationRequest::prompt("flux-dev", "test"))
            .await
            .unwrap();
        let (parent, binding) = limiters.limiter_for("fal", "fal-ai/flux/dev", None);
        assert_eq!(parent.limit(), 4);
        assert_eq!(binding.limit(), 4);
        assert_eq!(binding.in_flight(), 0);
        assert_eq!(provider.polled.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), provider.cancelled.notified())
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn generation_deadline_cancels_job_and_releases_permits() {
        let (dispatcher, provider, limiters) = queued_dispatcher(false);
        let error = dispatcher
            .with_generation_timeout(Duration::from_millis(30))
            .generate(&GenerationRequest::prompt("flux-dev", "test"))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            KalpaError::ProviderError { status: 504, .. }
        ));
        tokio::time::timeout(Duration::from_secs(1), provider.cancelled.notified())
            .await
            .unwrap();
        let (parent, binding) = limiters.limiter_for("fal", "fal-ai/flux/dev", None);
        assert_eq!(parent.in_flight(), 0);
        assert_eq!(binding.in_flight(), 0);
    }

    #[tokio::test]
    async fn caller_abort_cancels_job_and_releases_permits() {
        let (dispatcher, provider, limiters) = queued_dispatcher(false);
        let task = tokio::spawn(async move {
            dispatcher
                .generate(&GenerationRequest::prompt("flux-dev", "test"))
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while provider.polled.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(1), provider.cancelled.notified())
            .await
            .unwrap();
        let (parent, binding) = limiters.limiter_for("fal", "fal-ai/flux/dev", None);
        assert_eq!(parent.in_flight(), 0);
        assert_eq!(binding.in_flight(), 0);
    }
}
