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
    config: AimdConfig,
    limit: AtomicU32,
    in_flight: AtomicU32,
    /// EWMA of completion latency in millis; 0 = unset.
    baseline_ms: AtomicU64,
    last_decrease: Mutex<Option<Instant>>,
    notify: Notify,
}

impl AimdLimiter {
    /// Create a limiter from a seed config.
    pub fn new(config: AimdConfig) -> Arc<Self> {
        let initial = config.initial.clamp(config.min.max(1), config.max.max(1));
        Arc::new(Self {
            config,
            limit: AtomicU32::new(initial),
            in_flight: AtomicU32::new(0),
            baseline_ms: AtomicU64::new(0),
            last_decrease: Mutex::new(None),
            notify: Notify::new(),
        })
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
                self.notify.notified().await;
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
            let mut cur = self.limit.load(Ordering::Acquire);
            loop {
                let next = (cur + 1).min(self.config.max);
                if next == cur {
                    break;
                }
                match self.limit.compare_exchange(
                    cur,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
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
        let ms = latency.as_millis() as u64;
        if ms == 0 {
            return false;
        }
        let prev = self.baseline_ms.load(Ordering::Acquire);
        if prev == 0 {
            self.baseline_ms.store(ms, Ordering::Release);
            return false;
        }
        // EWMA (alpha = 1/8) for the running baseline.
        let ewma = (prev * 7 + ms) / 8;
        self.baseline_ms.store(ewma, Ordering::Release);
        (ms as f64) > (prev as f64) * self.config.latency_threshold
    }

    /// Multiplicative decrease, debounced by `cooldown`.
    fn decrease(&self) {
        {
            let mut last = self.last_decrease.lock().unwrap();
            let now = Instant::now();
            if let Some(t) = *last {
                if now.duration_since(t) < self.config.cooldown {
                    return; // within cooldown — count this burst once
                }
            }
            *last = Some(now);
        }
        let mut cur = self.limit.load(Ordering::Acquire);
        loop {
            let next = ((cur as f64 * self.config.decrease_factor).floor() as u32)
                .max(self.config.min);
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
    bindings: RwLock<HashMap<String, Arc<AimdLimiter>>>,
    providers: RwLock<HashMap<String, Arc<AimdLimiter>>>,
    provider_defaults: HashMap<String, AimdConfig>,
    global_default: AimdConfig,
}

fn binding_key(provider: &str, slug: &str, region: Option<&str>) -> String {
    format!("{provider}:{slug}:{}", region.unwrap_or(""))
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
        self.bindings
            .write()
            .unwrap()
            .insert(key, AimdLimiter::new(cfg));
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
        let l = AimdLimiter::new(AimdConfig { initial: 2, ..cfg() });
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
            AimdConfig { initial: 2, min: 1, max: 2, ..cfg() },
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
}
