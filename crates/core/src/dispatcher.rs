//! Generation dispatcher: resolve a model ref → AIMD-gate → drive the provider
//! lifecycle (sync inline, or async submit→poll) → return a unified response.
//!
//! M1 uses a single global limiter; M3 swaps in a per-binding `LimiterRegistry`
//! nested under a per-provider parent (the `generate` flow is unchanged).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::error::{KalpaError, KalpaResult};
use crate::generation::{
    GenerationRequest, GenerationResponse, ModelRef, SpeechRequest, TranscriptionRequest,
};
use crate::provider::{
    GenerationProvider, PollStatus, SpeechProvider, SubmitOutcome, TranscriptionProvider,
};
use crate::ratelimit::{AimdLimiter, LimiterRegistry};
use crate::registry::Registry;
use crate::retry::{is_rate_limit_rejection, retry_if, RetryConfig};

/// Routes generation requests to providers, gated by per-binding AIMD limiters
/// (each nested under a per-provider parent ceiling).
pub struct Dispatcher {
    registry: Registry,
    providers: HashMap<String, Arc<dyn GenerationProvider>>,
    speech: HashMap<String, Arc<dyn SpeechProvider>>,
    transcription: HashMap<String, Arc<dyn TranscriptionProvider>>,
    limiters: Arc<LimiterRegistry>,
    poll_interval: Duration,
    retry: RetryConfig,
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
            retry: RetryConfig::default(),
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
        self.poll_interval = interval;
        self
    }

    /// Override the submit-retry policy.
    ///
    /// Budget note: the default is 3 attempts with 500ms initial backoff and a
    /// 2.0 multiplier, so the *added* latency is at most 500ms + 1000ms = 1.5s
    /// (`max_backoff` never binds at 3 attempts). That fits well inside
    /// svstudio's 120s `MAX_SYNC_WAIT` and its ~120s poll budget. Raising
    /// `max_attempts` re-opens that question, and for a *synchronous* provider
    /// each attempt re-runs the whole generation, not just a submit.
    pub fn with_retry_config(mut self, retry: RetryConfig) -> Self {
        self.retry = retry;
        self
    }

    /// The model catalog backing this dispatcher (for `/v1/models`).
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Run a billable provider call, retrying only rate-limit rejections and
    /// feeding every rejection to the AIMD limiter.
    ///
    /// Backpressure and retry are complementary rather than alternatives: the
    /// limiter narrows the concurrency window for *subsequent* work, while the
    /// retry re-issues *this* request after a backoff. Without the retry a 429
    /// is a hard failure for the caller even though the limiter has already
    /// adapted.
    ///
    /// Both permits stay held across the backoff sleeps. That is deliberate —
    /// releasing them would let a queued request take the slot we are about to
    /// reuse, defeating the backoff.
    async fn submit_with_retry<T, F, Fut>(
        &self,
        binding: &Arc<AimdLimiter>,
        op: F,
    ) -> KalpaResult<T>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = KalpaResult<T>>,
    {
        retry_if(self.retry.clone(), is_rate_limit_rejection, || {
            let binding = Arc::clone(binding);
            let fut = op();
            async move {
                let result = fut.await;
                if let Err(e) = &result {
                    if is_rate_limit_rejection(e) {
                        binding.on_submit_rejected(None); // fast-loop MD
                    }
                }
                result
            }
        })
        .await
    }

    /// Resolve, gate, and run a single generation to a terminal result.
    pub async fn generate(&self, request: &GenerationRequest) -> KalpaResult<GenerationResponse> {
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

        // Two-level gate: parent (provider account ceiling) then binding (the
        // adaptive limiter). Both permits are held for the WHOLE unit of work;
        // AIMD signals are applied to the binding limiter only.
        let (parent, binding) = self.limiters.limiter_for(
            &resolved.binding.provider,
            &resolved.binding.provider_slug,
            resolved.binding.region.as_deref(),
        );
        let _parent_permit = parent.acquire().await;
        let permit = binding.acquire().await;
        let start = Instant::now();

        let outcome = self
            .submit_with_retry(&binding, || provider.submit(&preq))
            .await?;

        match outcome {
            SubmitOutcome::Sync(resp) => {
                binding.on_completed(&permit, start.elapsed());
                Ok(resp)
            }
            SubmitOutcome::Async(handle) => loop {
                match provider.poll(&handle).await? {
                    PollStatus::InQueue { position } => {
                        binding.observe_queue(position, start.elapsed());
                        tokio::time::sleep(self.poll_interval).await;
                    }
                    PollStatus::InProgress => {
                        tokio::time::sleep(self.poll_interval).await;
                    }
                    PollStatus::Completed(resp) => {
                        binding.on_completed(&permit, start.elapsed());
                        return Ok(resp);
                    }
                    PollStatus::Failed(msg) => {
                        binding.on_failed(false);
                        return Err(KalpaError::ProviderError {
                            status: 500,
                            message: msg,
                        });
                    }
                }
            },
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
        let _parent_permit = parent.acquire().await;
        let permit = binding.acquire().await;
        let start = Instant::now();
        let resp = self
            .submit_with_retry(&binding, || provider.synthesize(&req))
            .await?;
        binding.on_completed(&permit, start.elapsed());
        Ok(resp)
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
        let _parent_permit = parent.acquire().await;
        let permit = binding.acquire().await;
        let start = Instant::now();
        let resp = self
            .submit_with_retry(&binding, || provider.transcribe(&req))
            .await?;
        binding.on_completed(&permit, start.elapsed());
        Ok(resp)
    }
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

    /// Fails the first `fail_first` submits with `err`, then succeeds.
    struct FlakyFal {
        attempts: Arc<std::sync::atomic::AtomicU32>,
        fail_first: u32,
        err: fn() -> KalpaError,
    }

    #[async_trait]
    impl GenerationProvider for FlakyFal {
        fn name(&self) -> &str {
            "fal"
        }
        async fn submit(&self, req: &GenerationRequest) -> KalpaResult<SubmitOutcome> {
            let n = self
                .attempts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.fail_first {
                return Err((self.err)());
            }
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

    fn dispatcher_for(provider: Arc<dyn GenerationProvider>) -> Dispatcher {
        let mut providers: HashMap<String, Arc<dyn GenerationProvider>> = HashMap::new();
        providers.insert("fal".into(), provider);
        Dispatcher::new(
            Registry::with_defaults(),
            providers,
            Arc::new(LimiterRegistry::new(AimdConfig::default(), Map::new())),
        )
        // Keep the test fast: the policy under test is *which* errors retry,
        // not how long the backoff is.
        .with_retry_config(RetryConfig {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(4),
            multiplier: 2.0,
        })
    }

    #[tokio::test]
    async fn rate_limited_submit_is_retried_until_it_succeeds() {
        let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let d = dispatcher_for(Arc::new(FlakyFal {
            attempts: attempts.clone(),
            fail_first: 2,
            err: || KalpaError::RateLimited("slow down".into()),
        }));

        let resp = d
            .generate(&GenerationRequest::prompt("flux-dev", "a cat"))
            .await
            .unwrap();
        assert_eq!(resp.image_urls(), vec!["https://example/out.png"]);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retries_are_capped_by_max_attempts() {
        let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let d = dispatcher_for(Arc::new(FlakyFal {
            attempts: attempts.clone(),
            fail_first: u32::MAX,
            err: || KalpaError::RateLimited("slow down".into()),
        }));

        assert!(d
            .generate(&GenerationRequest::prompt("flux-dev", "a cat"))
            .await
            .is_err());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn server_errors_are_not_retried_at_the_submit_boundary() {
        // A 500 may arrive *after* the provider accepted the job. Retrying it
        // would submit a second billable generation and orphan the first, so
        // the submit path deliberately does not — even though `is_retryable`
        // classifies 500 as transient for read paths.
        let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let d = dispatcher_for(Arc::new(FlakyFal {
            attempts: attempts.clone(),
            fail_first: 1,
            err: || KalpaError::ProviderError {
                status: 500,
                message: "upstream boom".into(),
            },
        }));

        assert!(d
            .generate(&GenerationRequest::prompt("flux-dev", "a cat"))
            .await
            .is_err());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_429_delivered_as_provider_error_also_retries() {
        // Providers report rate limiting either way; only `RateLimited` used to
        // reach the AIMD signal.
        let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let d = dispatcher_for(Arc::new(FlakyFal {
            attempts: attempts.clone(),
            fail_first: 1,
            err: || KalpaError::ProviderError {
                status: 429,
                message: "quota".into(),
            },
        }));

        d.generate(&GenerationRequest::prompt("flux-dev", "a cat"))
            .await
            .unwrap();
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn auth_errors_fail_immediately() {
        let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let d = dispatcher_for(Arc::new(FlakyFal {
            attempts: attempts.clone(),
            fail_first: 1,
            err: || KalpaError::Auth("bad key".into()),
        }));

        assert!(d
            .generate(&GenerationRequest::prompt("flux-dev", "a cat"))
            .await
            .is_err());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
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
}
