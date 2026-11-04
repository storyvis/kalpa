//! AIMD concurrency limiter for provider calls.
//!
//! Limits *outstanding generations* (permit held start-of-work → end-of-work),
//! adapting to observed back-pressure instead of a hardcoded number. The single
//! "429 → halve" rule is split across the provider lifecycle into three signals
//! (see the design doc's AIMD section):
//!
//! - **submit 429/503** → multiplicative decrease (fast loop), cooldown-guarded
//! - **completion while saturated** → additive increase (slow loop)
//! - **rising time-to-result** → latency-gradient decrease (queue absorbs load
//!   without erroring; errors lag, latency leads)
//!
//! A `poll` 429 is *not* handled here — it backs off the per-job poll interval.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// Seed configuration for one limiter (one binding).
#[derive(Debug, Clone)]
pub struct AimdConfig {
    /// Starting concurrency limit.
    pub initial: u32,
    /// Floor — keeps liveness.
    pub min: u32,
    /// Ceiling — backstops the additive-increase / latency-blind case.
    pub max: u32,
    /// Multiplicative-decrease factor (e.g. 0.5).
    pub decrease_factor: f64,
    /// Debounce so a burst of decreases from one overload counts once.
    pub cooldown: Duration,
    /// Latency-gradient trip ratio (observed / baseline).
    /// Positive infinity disables latency-driven decreases; explicit
    /// backpressure and the concurrency bounds still apply.
    pub latency_threshold: f64,
}

impl Default for AimdConfig {
    fn default() -> Self {
        Self {
            initial: 4,
            min: 1,
            max: 64,
            decrease_factor: 0.5,
            cooldown: Duration::from_millis(500),
            latency_threshold: 2.0,
        }
    }
}

/// AIMD concurrency limiter for a single binding.
pub struct AimdLimiter {
    config: RwLock<AimdConfig>,
    limit: AtomicU32,
    in_flight: AtomicU32,
    /// EWMA of completion latency in millis; 0 = unset.
    baseline_ms: AtomicU64,
    last_decrease: Mutex<Option<Instant>>,
    notify: Notify,
}

impl AimdLimiter {
    /// Create a limiter from a seed config.
    pub fn new(mut config: AimdConfig) -> Arc<Self> {
        config.max = config.max.max(1);
        config.min = config.min.clamp(1, config.max);
        if !config.decrease_factor.is_finite() || !(0.0..1.0).contains(&config.decrease_factor) {
            config.decrease_factor = 0.5;
        }
        if config.latency_threshold.is_nan() || config.latency_threshold <= 1.0 {
            config.latency_threshold = 2.0;
        }
        let initial = config.initial.clamp(config.min, config.max);
        Arc::new(Self {
            config: RwLock::new(config),
            limit: AtomicU32::new(initial),
            in_flight: AtomicU32::new(0),
            baseline_ms: AtomicU64::new(0),
            last_decrease: Mutex::new(None),
            notify: Notify::new(),
        })
    }

    fn reconfigure(&self, config: AimdConfig) {
        let normalized = Self::new(config);
        let config = normalized.config.read().unwrap().clone();
        let mut current = self.config.write().unwrap();
        self.limit
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |limit| {
                Some(limit.clamp(config.min, config.max))
            })
            .unwrap();
        *current = config;
        self.notify.notify_waiters();
    }

    /// Current concurrency limit (for observability / logs).
    pub fn limit(&self) -> u32 {
        self.limit.load(Ordering::Acquire)
    }

    /// Current outstanding count.
    pub fn in_flight(&self) -> u32 {
        self.in_flight.load(Ordering::Acquire)
    }

    /// Acquire a permit, awaiting while `in_flight >= limit`. The permit is held
    /// for the whole unit of work and released (decrementing `in_flight`) on drop.
    pub async fn acquire(self: &Arc<Self>) -> Permit {
        loop {
            // Register before checking capacity: several releases before await
            // must wake several waiters, not coalesce into one stored notification.
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let limit = self.limit.load(Ordering::Acquire);
            let cur = self.in_flight.load(Ordering::Acquire);
            if cur < limit {
                if self
                    .in_flight
                    .compare_exchange(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    // Saturated iff we took the last slot at the ceiling.
                    let saturated = cur + 1 >= limit;
                    return Permit {
                        limiter: Arc::clone(self),
                        saturated,
                    };
                }
                // lost the race, retry
            } else {
                notified.await;
            }
        }
    }

    /// Submit-time back-pressure (429/503/timeout): multiplicative decrease.
    pub fn on_submit_rejected(&self, _retry_after: Option<Duration>) {
        self.decrease();
    }

    /// A generation completed successfully. Additive increase if it was acquired
    /// at the ceiling, unless the latency gradient has tripped.
    pub fn on_completed(&self, permit: &Permit, latency: Duration) {
        if self.observe_latency(latency) {
            // gradient tripped → decrease instead of increase
            self.decrease();
            return;
        }
        if permit.saturated {
            let config = self.config.read().unwrap();
            let mut cur = self.limit.load(Ordering::Acquire);
            loop {
                let next = cur.saturating_add(1).min(config.max);
                if next == cur {
                    break;
                }
                match self
                    .limit
                    .compare_exchange(cur, next, Ordering::AcqRel, Ordering::Acquire)
                {
                    Ok(_) => {
                        self.notify.notify_one();
                        break;
                    }
                    Err(observed) => cur = observed,
                }
            }
        }
    }

    /// A generation failed. Capacity-related failures decrease; others are neutral.
    pub fn on_failed(&self, capacity_related: bool) {
        if capacity_related {
            self.decrease();
        }
    }

    /// Record a queue observation (soft latency signal); decreases if a long
    /// time-in-queue trips the gradient.
    pub fn observe_queue(&self, _position: Option<u32>, elapsed: Duration) {
        if self.observe_latency(elapsed) {
            self.decrease();
        }
    }

    /// Update the latency EWMA and return whether the gradient has tripped.
    fn observe_latency(&self, latency: Duration) -> bool {
        let ms = latency.as_millis().min(u64::MAX as u128) as u64;
        if ms == 0 {
            return false;
        }
        let prev = self.baseline_ms.load(Ordering::Acquire);
        if prev == 0 {
            self.baseline_ms.store(ms, Ordering::Release);
            return false;
        }
        // EWMA (alpha = 1/8) for the running baseline.
        let ewma = ((u128::from(prev) * 7 + u128::from(ms)) / 8) as u64;
        self.baseline_ms.store(ewma, Ordering::Release);
        (ms as f64) > (prev as f64) * self.config.read().unwrap().latency_threshold
    }

    /// Multiplicative decrease, debounced by `cooldown`.
    fn decrease(&self) {
        let config = self.config.read().unwrap();
        {
            let mut last = self.last_decrease.lock().unwrap();
            let now = Instant::now();
            if let Some(t) = *last {
                if now.duration_since(t) < config.cooldown {
                    return; // within cooldown — count this burst once
                }
            }
            *last = Some(now);
        }
        let mut cur = self.limit.load(Ordering::Acquire);
        loop {
            let next = ((cur as f64 * config.decrease_factor).floor() as u32).max(config.min);
            if next == cur {
                break;
            }
            match self
                .limit
                .compare_exchange(cur, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(observed) => cur = observed,
            }
        }
    }
}

/// Seed for one binding's limiter (from `model_providers.rate_limit`).
#[derive(Debug, Clone)]
pub struct BindingSpec {
    pub provider: String,
    pub provider_slug: String,
    pub region: Option<String>,
    /// Per-binding AIMD seed; falls back to provider/global default when `None`.
    pub config: Option<AimdConfig>,
}

/// Registry of AIMD limiters: one per `(provider, slug, region)` binding, each
/// nested under a per-provider parent that guards the provider's shared account
/// quota. A call acquires the parent permit then the binding permit.
pub struct LimiterRegistry {
    bindings: RwLock<HashMap<(String, String, Option<String>), Arc<AimdLimiter>>>,
    providers: RwLock<HashMap<String, Arc<AimdLimiter>>>,
    provider_defaults: HashMap<String, AimdConfig>,
    global_default: AimdConfig,
}

fn binding_key(
    provider: &str,
    slug: &str,
    region: Option<&str>,
) -> (String, String, Option<String>) {
    (provider.into(), slug.into(), region.map(str::to_owned))
}

impl LimiterRegistry {
    /// Build an empty registry with default seeds.
    pub fn new(global_default: AimdConfig, provider_defaults: HashMap<String, AimdConfig>) -> Self {
        Self {
            bindings: RwLock::new(HashMap::new()),
            providers: RwLock::new(HashMap::new()),
            provider_defaults,
            global_default,
        }
    }

    /// Eagerly create one limiter per binding (and its provider parent).
    pub fn init_from_catalog(&self, specs: &[BindingSpec]) {
        for spec in specs {
            self.upsert_binding(spec);
        }
    }

    /// Seed resolution: binding config → provider default → global default.
    fn config_for(&self, provider: &str, explicit: Option<&AimdConfig>) -> AimdConfig {
        explicit
            .cloned()
            .or_else(|| self.provider_defaults.get(provider).cloned())
            .unwrap_or_else(|| self.global_default.clone())
    }

    /// Insert or replace a binding's limiter (hot reload on catalog change).
    pub fn upsert_binding(&self, spec: &BindingSpec) {
        let cfg = self.config_for(&spec.provider, spec.config.as_ref());
        let key = binding_key(&spec.provider, &spec.provider_slug, spec.region.as_deref());
        let mut bindings = self.bindings.write().unwrap();
        if let Some(limiter) = bindings.get(&key) {
            limiter.reconfigure(cfg);
        } else {
            bindings.insert(key, AimdLimiter::new(cfg));
        }
        drop(bindings);
        // Ensure the provider parent exists.
        self.provider_parent(&spec.provider);
    }

    /// Remove a binding's limiter.
    pub fn remove_binding(&self, provider: &str, slug: &str, region: Option<&str>) {
        self.bindings
            .write()
            .unwrap()
            .remove(&binding_key(provider, slug, region));
    }

    /// Get-or-create the per-provider parent limiter (shared account ceiling).
    fn provider_parent(&self, provider: &str) -> Arc<AimdLimiter> {
        if let Some(p) = self.providers.read().unwrap().get(provider) {
            return Arc::clone(p);
        }
        let mut w = self.providers.write().unwrap();
        Arc::clone(
            w.entry(provider.to_string())
                .or_insert_with(|| AimdLimiter::new(self.config_for(provider, None))),
        )
    }

    /// Resolve the `(parent, binding)` limiters for a binding, creating them
    /// lazily if absent.
    pub fn limiter_for(
        &self,
        provider: &str,
        slug: &str,
        region: Option<&str>,
    ) -> (Arc<AimdLimiter>, Arc<AimdLimiter>) {
        let parent = self.provider_parent(provider);
        let key = binding_key(provider, slug, region);
        if let Some(b) = self.bindings.read().unwrap().get(&key) {
            return (parent, Arc::clone(b));
        }
        let mut w = self.bindings.write().unwrap();
        let binding = Arc::clone(
            w.entry(key)
                .or_insert_with(|| AimdLimiter::new(self.config_for(provider, None))),
        );
        (parent, binding)
    }
}

/// RAII permit. Holds `in_flight` for the unit of work; releasing wakes a waiter.
pub struct Permit {
    limiter: Arc<AimdLimiter>,
    /// Whether this permit was acquired at the ceiling (drives additive increase).
    saturated: bool,
}

impl Permit {
    /// Whether this permit was acquired while the limiter was saturated.
    pub fn saturated(&self) -> bool {
        self.saturated
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.limiter.in_flight.fetch_sub(1, Ordering::AcqRel);
        self.limiter.notify.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AimdConfig {
        AimdConfig {
            initial: 8,
            min: 1,
            max: 32,
            decrease_factor: 0.5,
            cooldown: Duration::from_millis(50),
            latency_threshold: 2.0,
        }
    }

    #[tokio::test]
    async fn md_on_submit_rejected() {
        let l = AimdLimiter::new(cfg());
        assert_eq!(l.limit(), 8);
        l.on_submit_rejected(None);
        assert_eq!(l.limit(), 4);
    }

    #[tokio::test]
    async fn cooldown_debounces_burst() {
        let l = AimdLimiter::new(cfg());
        l.on_submit_rejected(None); // 8 -> 4
        l.on_submit_rejected(None); // within cooldown -> ignored
        assert_eq!(l.limit(), 4);
    }

    #[tokio::test]
    async fn ai_only_when_saturated() {
        let l = AimdLimiter::new(AimdConfig {
            initial: 2,
            ..cfg()
        });
        // Fill to the ceiling: two permits, second is saturated.
        let _p1 = l.acquire().await;
        let p2 = l.acquire().await;
        assert!(p2.saturated());
        l.on_completed(&p2, Duration::from_millis(10));
        assert_eq!(l.limit(), 3); // additive increase
    }

    #[tokio::test]
    async fn registry_shares_provider_parent() {
        let reg = LimiterRegistry::new(
            AimdConfig {
                initial: 2,
                min: 1,
                max: 2,
                ..cfg()
            },
            std::collections::HashMap::new(),
        );
        let (p1, b1) = reg.limiter_for("fal", "model-a", None);
        let (p2, b2) = reg.limiter_for("fal", "model-b", None);

        // Both bindings of one provider share the same parent ceiling…
        assert!(Arc::ptr_eq(&p1, &p2));
        // …but have distinct per-binding limiters.
        assert!(!Arc::ptr_eq(&b1, &b2));

        // The parent caps combined in-flight regardless of which binding is used.
        let _a = p1.acquire().await;
        let _b = p2.acquire().await;
        assert_eq!(p1.in_flight(), 2);
        assert_eq!(p1.limit(), 2);
    }

    #[tokio::test]
    async fn latency_gradient_decreases() {
        let l = AimdLimiter::new(cfg());
        let p = l.acquire().await;
        l.on_completed(&p, Duration::from_millis(100)); // sets baseline 100
                                                        // A much slower completion trips the gradient → decrease, no increase.
        let p2 = l.acquire().await;
        l.on_completed(&p2, Duration::from_millis(500));
        assert!(l.limit() < 8);
    }
    #[tokio::test]
    async fn infinite_latency_threshold_disables_only_latency_backpressure() {
        for reconfigure in [false, true] {
            let config = AimdConfig {
                latency_threshold: f64::INFINITY,
                ..cfg()
            };
            let limiter = if reconfigure {
                let limiter = AimdLimiter::new(cfg());
                limiter.reconfigure(config);
                limiter
            } else {
                AimdLimiter::new(config)
            };
            for latency in [Duration::from_millis(1), Duration::from_secs(60)] {
                let permit = limiter.acquire().await;
                limiter.on_completed(&permit, latency);
                drop(permit);
            }
            assert_eq!(
                limiter.limit(),
                8,
                "successful latency must not reduce capacity"
            );
            limiter.on_failed(true);
            assert_eq!(
                limiter.limit(),
                4,
                "explicit backpressure must still reduce capacity"
            );
        }
    }

    #[test]
    fn invalid_latency_thresholds_keep_the_default_gradient() {
        for latency_threshold in [f64::NAN, f64::NEG_INFINITY, -1.0, 0.0, 1.0] {
            let limiter = AimdLimiter::new(AimdConfig {
                latency_threshold,
                ..cfg()
            });
            assert!(!limiter.observe_latency(Duration::from_millis(100)));
            assert!(limiter.observe_latency(Duration::from_millis(500)));
        }
    }

    #[test]
    fn invalid_config_preserves_positive_bounded_capacity() {
        let limiter = AimdLimiter::new(AimdConfig {
            initial: 0,
            min: 10,
            max: 0,
            decrease_factor: f64::NAN,
            ..cfg()
        });
        assert!(limiter.limit() > 0);
        limiter.on_submit_rejected(None);
        assert!(limiter.limit() > 0);
    }

    #[test]
    fn zero_floor_cannot_remove_last_slot() {
        let limiter = AimdLimiter::new(AimdConfig {
            initial: 1,
            min: 0,
            max: 1,
            ..cfg()
        });
        limiter.on_submit_rejected(None);
        assert_eq!(limiter.limit(), 1);
    }

    #[tokio::test]
    async fn hot_reload_preserves_outstanding_permits() {
        let registry = LimiterRegistry::new(cfg(), HashMap::new());
        let (_, before) = registry.limiter_for("fal", "model", None);
        let permit = before.acquire().await;
        registry.upsert_binding(&BindingSpec {
            provider: "fal".into(),
            provider_slug: "model".into(),
            region: None,
            config: Some(AimdConfig {
                initial: 1,
                min: 1,
                max: 1,
                ..cfg()
            }),
        });
        let (_, after) = registry.limiter_for("fal", "model", None);
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(after.in_flight(), 1);
        assert_eq!(after.limit(), 1);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), after.acquire())
                .await
                .is_err()
        );
        drop(permit);
        let _permit = tokio::time::timeout(Duration::from_secs(1), after.acquire())
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_waiters_do_not_lose_releases() {
        let limiter = AimdLimiter::new(AimdConfig {
            initial: 4,
            min: 4,
            max: 4,
            ..cfg()
        });
        let mut tasks = Vec::new();
        for _ in 0..64 {
            let limiter = Arc::clone(&limiter);
            tasks.push(tokio::spawn(async move {
                for _ in 0..100 {
                    let _permit = limiter.acquire().await;
                    assert!(limiter.in_flight() <= 4);
                    tokio::task::yield_now().await;
                }
            }));
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            for task in tasks {
                task.await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(limiter.in_flight(), 0);
    }
}
