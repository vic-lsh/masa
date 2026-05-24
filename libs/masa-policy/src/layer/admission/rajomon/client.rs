use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use once_cell::sync::Lazy;
use tonic_core::CowGrpcMethod;

use crate::policy_params::PolicyParams;

// ── Client Token Bucket ─────────────────────────────────────────────────

/// Client-side token bucket for Rajomon rate limiting.
/// Single global counter matching the original Go implementation.
#[allow(missing_debug_implementations)]
pub struct ClientTokenBucket {
    tokens_left: AtomicU64,
    cached_prices: DashMap<CowGrpcMethod, u64>,
}

impl ClientTokenBucket {
    pub(super) fn new() -> Self {
        Self {
            tokens_left: AtomicU64::new(PolicyParams::global().rajomon.tokens_left_init),
            cached_prices: DashMap::new(),
        }
    }

    /// Try to acquire tokens for a method, returning the uniform-random token
    /// count attached to the outgoing request, or `None` if the bucket cannot
    /// cover the cached price for this method (rate limited).
    ///
    /// Paper (NSDI '25, §3.3 "Randomized Token Spending"): *"RAJOMON selects a
    /// uniform random number of tokens for each outgoing request, yielding a
    /// range of tokens attached to requests such that the number of dropped
    /// requests increases gradually as the price increases."* The paper
    /// explicitly rejects deterministic spending as making AQM "ineffective
    /// because the controller cannot distinguish priority across requests."
    ///
    /// Mirrors Go's `rajomon.go:278-282` randomization + `DeductTokens(tok)`
    /// flow: the randomized `tok` is the amount both deducted from the bucket
    /// and attached to the request. See `3rd_party/rajomon/RUST_PORT_ALIGNMENT.md`
    /// §0.3 / §10.3.
    pub fn try_acquire(&self, method: &CowGrpcMethod) -> Option<u64> {
        use rand::Rng;
        let price = self.cached_prices.get(method).map(|v| *v).unwrap_or(0);
        let mut rng = rand::thread_rng();
        // CAS loop: pick a new random tok on each retry, since a concurrent
        // replenish/deduct may have changed the bucket level.
        loop {
            let current = self.tokens_left.load(Ordering::Relaxed);
            if current < price {
                return None; // rate limited: bucket can't cover the price
            }
            // Paper: uniform random in [0, current]. Matches Go's
            // fastrand.Int63n(tok) semantics (inclusive on 0, exclusive on
            // current-when-current>0) up to a one-token edge case that does
            // not matter in practice.
            let tok = if current == 0 {
                0
            } else {
                rng.gen_range(0..=current)
            };
            if tok < price {
                // Rate-limited: the randomized draw came in below the price.
                // Return None without deducting so the client reports this
                // as a drop rather than "used tokens for a dropped request."
                return None;
            }
            if self
                .tokens_left
                .compare_exchange_weak(current, current - tok, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return Some(tok);
            }
        }
    }

    /// Current token balance (used by callers to pick a uniform random token value).
    pub fn tokens_left(&self) -> u64 {
        self.tokens_left.load(Ordering::Relaxed)
    }

    /// Deduct n tokens from the bucket (saturating at 0). Matches Go's DeductTokens.
    pub fn deduct(&self, n: u64) {
        loop {
            let current = self.tokens_left.load(Ordering::Relaxed);
            let new_val = current.saturating_sub(n);
            if self
                .tokens_left
                .compare_exchange_weak(current, new_val, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    /// Update the cached price for a method (called when response header received).
    pub fn update_price(&self, method: &CowGrpcMethod, price: u64) {
        self.cached_prices.insert(method.clone(), price);
    }

    /// Log the current bucket level and per-method cached prices. Called
    /// periodically by the replenishment worker so the user can observe
    /// whether the client-side rate limiter is the bottleneck.
    fn log_state(&self) {
        let tokens = self.tokens_left.load(Ordering::Relaxed);
        if self.cached_prices.is_empty() {
            log::info!(
                "Rajomon client tokens_left: {} (no cached prices yet)",
                tokens
            );
        } else {
            let parts: Vec<String> = self
                .cached_prices
                .iter()
                .map(|e| {
                    format!(
                        "{}::{}: {}",
                        e.key().service(),
                        e.key().method(),
                        *e.value()
                    )
                })
                .collect();
            log::info!(
                "Rajomon client tokens_left: {}, cached_prices: {}",
                tokens,
                parts.join(", ")
            );
        }
    }

    /// Replenish token pool by `token_update_step`.
    pub fn replenish(&self) {
        let p = &PolicyParams::global().rajomon;
        loop {
            let current = self.tokens_left.load(Ordering::Relaxed);
            let new_val = current.saturating_add(p.token_update_step);
            if self
                .tokens_left
                .compare_exchange_weak(current, new_val, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    /// Start the background replenishment worker.
    pub fn ensure_worker_started() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static REPLENISH_WORKER_STARTED: AtomicBool = AtomicBool::new(false);
        if !REPLENISH_WORKER_STARTED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async {
                use rand_distr::{Distribution, Exp};
                let rate = 1.0 / PolicyParams::global().rajomon.token_update_rate_ms as f64;
                let dist = Exp::new(rate).expect("Exp::new failed");
                let mut log_tick: u32 = 0;
                loop {
                    let sleep_ms: f64 = dist.sample(&mut rand::thread_rng());
                    let sleep_ms_clamped = sleep_ms.max(0.1).min(10_000.0);
                    tokio::time::sleep(Duration::from_secs_f64(sleep_ms_clamped / 1000.0)).await;
                    CLIENT_TOKEN_BUCKET.replenish();
                    log_tick += 1;
                    // At a 10ms mean tick, 500 ticks ≈ 5s — same cadence as
                    // RajomonSharedState::log_pricing_tables on the server side.
                    if log_tick >= 500 {
                        log_tick = 0;
                        CLIENT_TOKEN_BUCKET.log_state();
                    }
                }
            });
        }
    }
}

#[allow(missing_docs)]
pub static CLIENT_TOKEN_BUCKET: Lazy<ClientTokenBucket> = Lazy::new(|| ClientTokenBucket::new());
