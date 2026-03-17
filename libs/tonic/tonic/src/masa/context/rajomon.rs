use crate::{CowGrpcMethod, Status};
#[cfg(feature = "rajomon")]
use dashmap::DashMap;
#[cfg(not(feature = "rajomon"))]
use masa_core::Context;
#[cfg(feature = "rajomon")]
use masa_core::Context;
#[cfg(feature = "rajomon")]
use once_cell::sync::Lazy;
#[cfg(feature = "rajomon")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "rajomon")]
use std::time::Duration;

// ── Rajomon Tunable Parameters ──
// Aligned with the original Go implementation (3rd_party/rajomon/).
// All defaults match the original unless noted.

// Overload detection
#[cfg(feature = "rajomon")]
const PRICE_UPDATE_RATE_MS: u64 = 10; // original: priceUpdateRate (10ms)
#[cfg(feature = "rajomon")]
const LATENCY_THRESHOLD_US: u64 = 1_000; // original: latencyThreshold (0)

// Price update (step strategy)
#[cfg(feature = "rajomon")]
const PRICE_STEP_UP: u64 = 8; // paper: +8 per tick when congested
#[cfg(feature = "rajomon")]
const PRICE_STEP_DOWN: u64 = 1; // paper: -1 per tick when below half-threshold
#[cfg(feature = "rajomon")]
const INIT_PRICE: u64 = 0; // original: initprice (0)

// Price propagation
#[cfg(feature = "rajomon")]
const PRICE_FREQ: u64 = 1; // original: priceFreq (5) — send price every 1/N requests

// Client-side token bucket
#[cfg(feature = "rajomon")]
const TOKENS_LEFT_INIT: u64 = 10; // original: tokensLeft (10)
#[cfg(feature = "rajomon")]
const TOKEN_UPDATE_RATE_MS: u64 = 10; // original: tokenUpdateRate (10ms)
#[cfg(feature = "rajomon")]
const TOKEN_UPDATE_STEP: u64 = 5; // original: tokenUpdateStep (1)
#[cfg(feature = "rajomon")]
const MAX_TOKEN: u64 = 100; // original: maxToken (10)

/// Global Rajomon state shared across all request handlers.
#[cfg(feature = "rajomon")]
pub static RAJOMON_STATE: Lazy<RajomonSharedState> = Lazy::new(|| RajomonSharedState::new());

/// Global queue statistics for the current time window.
/// Tracks the maximum queue latency observed since the last tick.
#[cfg(feature = "rajomon")]
pub struct QueueStats {
    /// Maximum per-request queue latency (us) observed since the last tick.
    pub window_max: AtomicU64,
}

#[cfg(feature = "rajomon")]
impl QueueStats {
    fn new() -> Self {
        Self {
            window_max: AtomicU64::new(0),
        }
    }
}

#[cfg(feature = "rajomon")]
impl std::fmt::Debug for QueueStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueueStats")
            .field("window_max", &self.window_max.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(feature = "rajomon")]
#[derive(Debug)]
#[allow(missing_docs)]
pub struct RajomonSharedState {
    /// Single global own price for the entire server.
    pub own_price: AtomicU64,
    /// Key: (parent_method, child_method) -> price
    pub downstream_prices: DashMap<(CowGrpcMethod, CowGrpcMethod), u64>,
    /// Key: parent_method -> max of all children's prices
    pub max_downstream_for_method: DashMap<CowGrpcMethod, u64>,
    /// Global queue stats (single instance, not per-method).
    pub queue_stats: QueueStats,
}

#[cfg(feature = "rajomon")]
impl RajomonSharedState {
    pub(crate) fn new() -> Self {
        Self {
            own_price: AtomicU64::new(INIT_PRICE),
            downstream_prices: DashMap::new(),
            max_downstream_for_method: DashMap::new(),
            queue_stats: QueueStats::new(),
        }
    }

    /// Accumulated price = max(own_price, downstream_price).
    /// Original Go: priceAggregation="maximal" -> max(ownPrice, downstreamPrice).
    pub fn accumulated_price(&self, method: &CowGrpcMethod) -> u64 {
        let own = self.own_price.load(Ordering::Relaxed);
        let downstream = self
            .max_downstream_for_method
            .get(method)
            .map(|v| *v)
            .unwrap_or(0);
        std::cmp::max(own, downstream)
    }

    /// Price to charge for calling a child method (cached from child's response).
    /// Looks up any entry where the child method appears as the second element of the key.
    pub fn child_price(&self, child_method: &CowGrpcMethod) -> u64 {
        self.downstream_prices
            .iter()
            .filter(|e| e.key().1 == *child_method)
            .map(|e| *e.value())
            .next()
            .unwrap_or(0)
    }

    /// Step price update algorithm with hysteresis band.
    /// if congestion (above threshold):         ownPrice += PRICE_STEP_UP
    /// else if below half-threshold:            ownPrice -= PRICE_STEP_DOWN
    /// else (between half and full threshold):  hold steady
    pub(crate) fn update_prices(&self) {
        let max_us = self.queue_stats.window_max.swap(0, Ordering::Relaxed);
        let own = self.own_price.load(Ordering::Relaxed);
        let new_price = if max_us > LATENCY_THRESHOLD_US {
            own + PRICE_STEP_UP
        } else if own > 0 && max_us < LATENCY_THRESHOLD_US / 2 {
            own - PRICE_STEP_DOWN
        } else {
            own
        };
        self.own_price.store(new_price, Ordering::Relaxed);
    }

    fn log_pricing_tables(&self) {
        log::info!(
            "Rajomon own_price: {}",
            self.own_price.load(Ordering::Relaxed)
        );
        if !self.downstream_prices.is_empty() {
            let parts: Vec<String> = self
                .downstream_prices
                .iter()
                .map(|e| {
                    format!(
                        "{}::{}->{}::{}: {}",
                        e.key().0.service(),
                        e.key().0.method(),
                        e.key().1.service(),
                        e.key().1.method(),
                        *e.value()
                    )
                })
                .collect();
            log::info!("Rajomon downstream_prices: {}", parts.join(", "));
        }
        if !self.max_downstream_for_method.is_empty() {
            let parts: Vec<String> = self
                .max_downstream_for_method
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
            log::info!("Rajomon max_downstream_for_method: {}", parts.join(", "));
        }
        log::info!(
            "Rajomon queue_latency (window_max): {} us",
            self.queue_stats.window_max.load(Ordering::Relaxed)
        );
    }

    // Helper to start the background worker once
    pub(super) fn ensure_worker_started() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WORKER_STARTED: AtomicBool = AtomicBool::new(false);
        if !WORKER_STARTED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async {
                let mut interval =
                    tokio::time::interval(Duration::from_millis(PRICE_UPDATE_RATE_MS));
                let mut log_tick: u32 = 0;
                loop {
                    interval.tick().await;
                    RAJOMON_STATE.update_prices();
                    log_tick += 1;
                    // At 10ms tick rate, 500 ticks = 5s logging interval
                    if log_tick >= 500 {
                        log_tick = 0;
                        RAJOMON_STATE.log_pricing_tables();
                    }
                }
            });
        }
    }
}

#[derive(Debug)]
pub(crate) struct RajomonHandler {
    #[cfg(feature = "rajomon")]
    rpc: CowGrpcMethod,
    #[cfg(feature = "rajomon")]
    should_drop: bool,
    /// Remaining token budget for this request, shared across fan-out branches.
    #[cfg(feature = "rajomon")]
    remaining_tokens: AtomicU64,
    /// Inbound token count from the request context (for deterministic price propagation).
    #[cfg(feature = "rajomon")]
    inbound_tokens: AtomicU64,
}

impl Default for RajomonHandler {
    fn default() -> Self {
        Self {
            #[cfg(feature = "rajomon")]
            rpc: CowGrpcMethod::new("", ""),
            #[cfg(feature = "rajomon")]
            should_drop: false,
            #[cfg(feature = "rajomon")]
            remaining_tokens: AtomicU64::new(0),
            #[cfg(feature = "rajomon")]
            inbound_tokens: AtomicU64::new(0),
        }
    }
}

impl RajomonHandler {
    pub(crate) fn new(rpc: CowGrpcMethod) -> Self {
        #[cfg(feature = "rajomon")]
        {
            RajomonSharedState::ensure_worker_started();
            Self {
                rpc,
                should_drop: false,
                remaining_tokens: AtomicU64::new(0),
                inbound_tokens: AtomicU64::new(0),
            }
        }
        #[cfg(not(feature = "rajomon"))]
        {
            let _ = rpc;
            Self {}
        }
    }

    /// Check inbound request tokens against accumulated price.
    /// Original: tokenleft = tokens - price (deduction at each hop).
    #[cfg(feature = "rajomon")]
    pub(crate) fn check_inbound(&mut self, ctx: &mut Context) -> bool {
        let price = RAJOMON_STATE.accumulated_price(&self.rpc);
        self.inbound_tokens.store(ctx.tokens(), Ordering::Relaxed);
        if ctx.tokens() < price {
            self.should_drop = true;
            true
        } else {
            self.remaining_tokens
                .store(ctx.tokens() - price, Ordering::Relaxed);
            false
        }
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn check_inbound(&mut self, _ctx: &mut Context) -> bool {
        false
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn should_drop(&self) -> bool {
        self.should_drop
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn should_drop(&self) -> bool {
        false
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn remaining_tokens(&self) -> u64 {
        self.remaining_tokens.load(Ordering::Relaxed)
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn remaining_tokens(&self) -> u64 {
        100 // default token budget when rajomon disabled
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn check_outbound(
        &self,
        child_method: &CowGrpcMethod,
        _ctx: &Context,
    ) -> Result<(), Status> {
        let price = RAJOMON_STATE.child_price(child_method);
        let current = self.remaining_tokens.load(Ordering::Relaxed);
        if current < price {
            return Err(self.issue_error(Some(child_method)));
        }
        Ok(())
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn check_outbound(
        &self,
        _child_method: &CowGrpcMethod,
        _ctx: &Context,
    ) -> Result<(), Status> {
        Ok(())
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn issue_error(&self, child_method: Option<&CowGrpcMethod>) -> Status {
        let mut msg = format!(
            "/EarlyReturn?src={}::{}",
            self.rpc.service(),
            self.rpc.method()
        );

        if let Some(child) = child_method {
            msg.push_str(&format!(
                "?last_rpc={}::{}",
                child.service(),
                child.method()
            ));
        }

        // Keep "Insufficient Rajomon Tokens" for backward compatibility in assertions
        msg.push_str(" Insufficient Rajomon Tokens");

        Status::resource_exhausted(msg)
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn issue_error(&self, _child_method: Option<&CowGrpcMethod>) -> Status {
        Status::resource_exhausted("Rajomon disabled")
    }

    /// Update the global window max with this poll's individual queue latency.
    /// Each poll measures time-in-run-queue for that specific wakeup; taking the max
    /// across polls (not accumulating) means the threshold is compared against a single
    /// scheduler wait, not the sum of all waits across the request lifetime.
    #[cfg(feature = "rajomon")]
    pub(crate) fn track_queue_delay(&self) {
        let q_lat_us = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        RAJOMON_STATE
            .queue_stats
            .window_max
            .fetch_max(q_lat_us, Ordering::Relaxed);
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn track_queue_delay(&self) {}

    /// No-op: window_max is updated per-poll in track_queue_delay.
    #[cfg(feature = "rajomon")]
    pub(crate) fn finalize_queue_delay(&self) {}

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn finalize_queue_delay(&self) {}

    #[cfg(feature = "rajomon")]
    pub(crate) fn update_cache_from_response(
        &self,
        child_method: &CowGrpcMethod,
        metadata: &crate::metadata::MetadataMap,
    ) {
        if let Some(price_header) = metadata.get("x-masa-rajomon-price") {
            if let Ok(price_str) = price_header.to_str() {
                if let Ok(price) = price_str.parse::<u64>() {
                    // Store per-child price keyed by (parent, child)
                    RAJOMON_STATE
                        .downstream_prices
                        .insert((self.rpc.clone(), child_method.clone()), price);
                    // Recompute max for this parent method across all children
                    let max_price = RAJOMON_STATE
                        .downstream_prices
                        .iter()
                        .filter(|e| e.key().0 == self.rpc)
                        .map(|e| *e.value())
                        .max()
                        .unwrap_or(0);
                    RAJOMON_STATE
                        .max_downstream_for_method
                        .insert(self.rpc.clone(), max_price);
                }
            }
        }
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn update_cache_from_response(
        &self,
        _child_method: &CowGrpcMethod,
        _metadata: &crate::metadata::MetadataMap,
    ) {
    }

    /// Deterministic price propagation: send price when inbound_tokens % PRICE_FREQ == 0.
    /// Original Go: tok % priceFreq == 0.
    #[cfg(feature = "rajomon")]
    fn should_propagate_price(&self) -> bool {
        let tokens = self.inbound_tokens.load(Ordering::Relaxed);
        tokens % PRICE_FREQ == 0
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn inject_price_to_response<T>(
        &self,
        result: &mut Result<crate::Response<T>, Status>,
    ) {
        if !self.should_propagate_price() {
            return;
        }
        let price = RAJOMON_STATE.accumulated_price(&self.rpc);
        if let Ok(value) = crate::metadata::MetadataValue::try_from(price.to_string()) {
            match result {
                Ok(resp) => {
                    resp.metadata_mut().insert("x-masa-rajomon-price", value);
                }
                Err(status) => {
                    status.metadata_mut().insert("x-masa-rajomon-price", value);
                }
            }
        }
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn inject_price_to_response<T>(
        &self,
        _result: &mut Result<crate::Response<T>, Status>,
    ) {
    }
}

/// Client-side token bucket for Rajomon rate limiting.
/// Single global counter matching the original Go implementation.
#[cfg(feature = "rajomon")]
#[allow(missing_debug_implementations)]
pub struct ClientTokenBucket {
    tokens_left: AtomicU64,
    cached_prices: DashMap<CowGrpcMethod, u64>,
}

#[cfg(feature = "rajomon")]
impl ClientTokenBucket {
    fn new() -> Self {
        Self {
            tokens_left: AtomicU64::new(TOKENS_LEFT_INIT),
            cached_prices: DashMap::new(),
        }
    }

    /// Try to acquire tokens for a method. Returns the actual token balance
    /// if successful, or None if the pool is insufficient (rate limited).
    pub fn try_acquire(&self, method: &CowGrpcMethod) -> Option<u64> {
        let price = self.cached_prices.get(method).map(|v| *v).unwrap_or(0);
        // CAS loop to deduct price from global pool
        loop {
            let current = self.tokens_left.load(Ordering::Relaxed);
            if current < price {
                return None; // rate limited
            }
            if self
                .tokens_left
                .compare_exchange_weak(
                    current,
                    current - price,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Some(current); // return actual token balance
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

    /// Replenish token pool by TOKEN_UPDATE_STEP, capped at MAX_TOKEN.
    /// Cap matches the maxToken field in Go (natural bound via per-request deductions).
    pub fn replenish(&self) {
        loop {
            let current = self.tokens_left.load(Ordering::Relaxed);
            let new_val = (current + TOKEN_UPDATE_STEP).min(MAX_TOKEN);
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
                let rate = 1.0 / TOKEN_UPDATE_RATE_MS as f64; // events per ms
                let dist = Exp::new(rate).expect("Exp::new failed");
                loop {
                    let sleep_ms: f64 = dist.sample(&mut rand::thread_rng());
                    let sleep_ms_clamped = sleep_ms.max(0.1).min(10_000.0);
                    tokio::time::sleep(Duration::from_secs_f64(sleep_ms_clamped / 1000.0)).await;
                    CLIENT_TOKEN_BUCKET.replenish();
                }
            });
        }
    }
}

#[cfg(feature = "rajomon")]
#[allow(missing_docs)]
pub static CLIENT_TOKEN_BUCKET: Lazy<ClientTokenBucket> = Lazy::new(|| ClientTokenBucket::new());

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    /// Mutex to serialize tests that modify the global RAJOMON_STATE.own_price,
    /// since it's a single global value shared across all test threads.
    #[cfg(feature = "rajomon")]
    static GLOBAL_STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ── A. Price Update Algorithm Tests (Step Strategy) ──

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_step_price_increase_on_congestion() {
        let state = RajomonSharedState::new();
        state.queue_stats.window_max.store(20000, Ordering::Relaxed);
        state.update_prices();
        assert_eq!(
            state.own_price.load(Ordering::Relaxed),
            INIT_PRICE + PRICE_STEP_UP
        );

        // Second tick, still congested
        state.queue_stats.window_max.store(15000, Ordering::Relaxed);
        state.update_prices();
        assert_eq!(
            state.own_price.load(Ordering::Relaxed),
            INIT_PRICE + 2 * PRICE_STEP_UP
        );
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_step_price_decrease_no_congestion() {
        let state = RajomonSharedState::new();
        state.own_price.store(5, Ordering::Relaxed);
        state.update_prices();
        assert_eq!(state.own_price.load(Ordering::Relaxed), 4);
        state.update_prices();
        assert_eq!(state.own_price.load(Ordering::Relaxed), 3);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_price_floor_at_zero() {
        let state = RajomonSharedState::new();
        state.own_price.store(1, Ordering::Relaxed);
        state.update_prices();
        assert_eq!(state.own_price.load(Ordering::Relaxed), 0);
        state.update_prices();
        assert_eq!(state.own_price.load(Ordering::Relaxed), 0);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_hysteresis_band_holds_price() {
        let state = RajomonSharedState::new();
        state.own_price.store(10, Ordering::Relaxed);
        // Set window_max to midpoint: between half-threshold and threshold
        state
            .queue_stats
            .window_max
            .store(LATENCY_THRESHOLD_US * 3 / 4, Ordering::Relaxed);
        state.update_prices();
        // Should hold, not increase or decrease
        assert_eq!(state.own_price.load(Ordering::Relaxed), 10);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_price_increase_uses_constant_step_not_proportional() {
        let state = RajomonSharedState::new();

        // Mild congestion (above threshold)
        state.own_price.store(0, Ordering::Relaxed);
        state.queue_stats.window_max.store(15000, Ordering::Relaxed);
        state.update_prices();
        let price_after_mild = state.own_price.load(Ordering::Relaxed);

        // Severe congestion
        state.own_price.store(0, Ordering::Relaxed);
        state
            .queue_stats
            .window_max
            .store(100_000, Ordering::Relaxed);
        state.update_prices();
        let price_after_severe = state.own_price.load(Ordering::Relaxed);

        assert_eq!(price_after_mild, price_after_severe);
        assert_eq!(price_after_mild, PRICE_STEP_UP);
    }

    // ── B. Queue Delay Signal Tests (Window Max) ──

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_queue_stats_tracks_maximum_not_average() {
        let state = RajomonSharedState::new();
        state
            .queue_stats
            .window_max
            .fetch_max(100, Ordering::Relaxed);
        state
            .queue_stats
            .window_max
            .fetch_max(5000, Ordering::Relaxed);
        state
            .queue_stats
            .window_max
            .fetch_max(200, Ordering::Relaxed);
        assert_eq!(state.queue_stats.window_max.load(Ordering::Relaxed), 5000);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_window_max_resets_after_price_update() {
        let state = RajomonSharedState::new();
        state.queue_stats.window_max.store(5000, Ordering::Relaxed);
        state.update_prices();
        assert_eq!(state.queue_stats.window_max.load(Ordering::Relaxed), 0);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_no_ewma_smoothing() {
        let state = RajomonSharedState::new();
        state.queue_stats.window_max.store(50000, Ordering::Relaxed);
        state.update_prices();
        assert_eq!(state.own_price.load(Ordering::Relaxed), PRICE_STEP_UP);

        // Next window: no latency observed (window_max already 0 from swap)
        state.update_prices();
        assert_eq!(
            state.own_price.load(Ordering::Relaxed),
            PRICE_STEP_UP - PRICE_STEP_DOWN
        );
    }

    // ── C. Price Aggregation Tests (Maximal Strategy) ──

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_accumulated_price_is_max_not_sum() {
        let state = RajomonSharedState::new();
        let method = CowGrpcMethod::new("svc", "method");

        // own=5, downstream=3 -> max=5
        state.own_price.store(5, Ordering::Relaxed);
        state.max_downstream_for_method.insert(method.clone(), 3);
        assert_eq!(state.accumulated_price(&method), 5);

        // own=5, downstream=10 -> max=10
        state.max_downstream_for_method.insert(method.clone(), 10);
        assert_eq!(state.accumulated_price(&method), 10);

        // own=0, no downstream -> 0
        state.own_price.store(0, Ordering::Relaxed);
        let method2 = CowGrpcMethod::new("svc", "other");
        assert_eq!(state.accumulated_price(&method2), 0);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_accumulated_price_own_only_no_downstream() {
        let state = RajomonSharedState::new();
        state.own_price.store(7, Ordering::Relaxed);
        let method = CowGrpcMethod::new("svc", "method");
        assert_eq!(state.accumulated_price(&method), 7);
    }

    // ── D. Token Consumption Tests (Deduction at Each Hop) ──

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_check_inbound_deducts_tokens() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        let method = CowGrpcMethod::new("svc", "method");
        let mut handler = RajomonHandler::new(method);
        RAJOMON_STATE.own_price.store(3, Ordering::Relaxed);
        // Clear any downstream for this method
        RAJOMON_STATE
            .max_downstream_for_method
            .remove(&CowGrpcMethod::new("svc", "method"));

        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(20).build();
        let dropped = handler.check_inbound(&mut ctx);
        assert!(!dropped);
        assert_eq!(handler.remaining_tokens(), 17); // 20 - 3 = 17
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_check_inbound_rejects_insufficient_tokens() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        let method = CowGrpcMethod::new("svc", "reject_method");
        let mut handler = RajomonHandler::new(method);
        RAJOMON_STATE.own_price.store(100, Ordering::Relaxed);

        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(10).build();
        let dropped = handler.check_inbound(&mut ctx);
        assert!(dropped);
        assert!(handler.should_drop());
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_check_inbound_accepts_exact_tokens() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        let method = CowGrpcMethod::new("svc", "exact_method");
        let mut handler = RajomonHandler::new(method);
        RAJOMON_STATE.own_price.store(10, Ordering::Relaxed);
        RAJOMON_STATE
            .max_downstream_for_method
            .remove(&CowGrpcMethod::new("svc", "exact_method"));

        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(10).build();
        let dropped = handler.check_inbound(&mut ctx);
        assert!(!dropped);
        assert_eq!(handler.remaining_tokens(), 0); // 10 - 10 = 0
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_check_inbound_tokens_zero_price_zero() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        let method = CowGrpcMethod::new("svc", "zero_method");
        let mut handler = RajomonHandler::new(method);
        RAJOMON_STATE.own_price.store(0, Ordering::Relaxed);

        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(0).build();
        let dropped = handler.check_inbound(&mut ctx);
        assert!(!dropped);
        assert_eq!(handler.remaining_tokens(), 0);
    }

    // ── E. Downstream Price Tests (Max Recomputation, Not Ratchet) ──

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_downstream_price_can_decrease() {
        let parent = CowGrpcMethod::new("svc", "parent");
        let child_x = CowGrpcMethod::new("svc", "X");
        let child_y = CowGrpcMethod::new("svc", "Y");

        let state = RajomonSharedState::new();

        // Set X=15 -> max=15
        state
            .downstream_prices
            .insert((parent.clone(), child_x.clone()), 15);
        let max = state
            .downstream_prices
            .iter()
            .filter(|e| e.key().0 == parent)
            .map(|e| *e.value())
            .max()
            .unwrap_or(0);
        state.max_downstream_for_method.insert(parent.clone(), max);
        assert_eq!(*state.max_downstream_for_method.get(&parent).unwrap(), 15);

        // Set Y=5 -> max still 15
        state
            .downstream_prices
            .insert((parent.clone(), child_y.clone()), 5);
        let max = state
            .downstream_prices
            .iter()
            .filter(|e| e.key().0 == parent)
            .map(|e| *e.value())
            .max()
            .unwrap_or(0);
        state.max_downstream_for_method.insert(parent.clone(), max);
        assert_eq!(*state.max_downstream_for_method.get(&parent).unwrap(), 15);

        // X drops to 3 -> max should now be 5 (from Y), NOT still 15
        state
            .downstream_prices
            .insert((parent.clone(), child_x.clone()), 3);
        let max = state
            .downstream_prices
            .iter()
            .filter(|e| e.key().0 == parent)
            .map(|e| *e.value())
            .max()
            .unwrap_or(0);
        state.max_downstream_for_method.insert(parent.clone(), max);
        assert_eq!(*state.max_downstream_for_method.get(&parent).unwrap(), 5);

        // Set Y=20 -> max=20
        state
            .downstream_prices
            .insert((parent.clone(), child_y.clone()), 20);
        let max = state
            .downstream_prices
            .iter()
            .filter(|e| e.key().0 == parent)
            .map(|e| *e.value())
            .max()
            .unwrap_or(0);
        state.max_downstream_for_method.insert(parent.clone(), max);
        assert_eq!(*state.max_downstream_for_method.get(&parent).unwrap(), 20);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_child_price_default_zero() {
        let state = RajomonSharedState::new();
        let method = CowGrpcMethod::new("svc", "method");
        assert_eq!(state.child_price(&method), 0);
    }

    // ── F. Client Token Bucket Tests (Single Global Counter) ──

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_client_token_bucket_single_global_counter() {
        let bucket = ClientTokenBucket::new();
        let method_a = CowGrpcMethod::new("svc", "A");
        let method_b = CowGrpcMethod::new("svc", "B");

        let tok_a = bucket.try_acquire(&method_a);
        assert!(tok_a.is_some());
        assert_eq!(tok_a.unwrap(), TOKENS_LEFT_INIT);

        let tok_b = bucket.try_acquire(&method_b);
        assert!(tok_b.is_some());
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_client_returns_actual_balance_not_random() {
        let bucket = ClientTokenBucket::new();
        let method = CowGrpcMethod::new("svc", "method");

        let tok = bucket.try_acquire(&method).unwrap();
        assert_eq!(tok, TOKENS_LEFT_INIT);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_client_replenish_caps_at_max() {
        let bucket = ClientTokenBucket::new();
        // Replenish enough times to reach MAX_TOKEN from TOKENS_LEFT_INIT
        for _ in 0..((MAX_TOKEN - TOKENS_LEFT_INIT) / TOKEN_UPDATE_STEP + 1) {
            bucket.replenish();
        }
        let tok = bucket.try_acquire(&CowGrpcMethod::new("svc", "m")).unwrap();
        assert_eq!(tok, MAX_TOKEN);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_client_rate_limits_when_insufficient() {
        let bucket = ClientTokenBucket::new();
        let method = CowGrpcMethod::new("svc", "method");
        bucket.update_price(&method, MAX_TOKEN + 1);

        let result = bucket.try_acquire(&method);
        assert!(result.is_none());
    }

    // ── G. Price Propagation Tests (Deterministic) ──

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_price_propagation_deterministic() {
        // With PRICE_FREQ=1, every request propagates price (N % 1 == 0 for all N).
        let handler = RajomonHandler::new(CowGrpcMethod::new("svc", "m"));
        handler.inbound_tokens.store(5, Ordering::Relaxed);
        assert!(handler.should_propagate_price()); // 5 % 1 == 0

        handler.inbound_tokens.store(3, Ordering::Relaxed);
        assert!(handler.should_propagate_price()); // 3 % 1 == 0

        handler.inbound_tokens.store(0, Ordering::Relaxed);
        assert!(handler.should_propagate_price()); // 0 % 1 == 0
    }

    // ── H. End-to-End Algorithmic Equivalence Tests ──

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_load_shedding_returns_correct_remaining_tokens() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        RAJOMON_STATE.own_price.store(7, Ordering::Relaxed);
        let method = CowGrpcMethod::new("svc", "Foo_ls");
        RAJOMON_STATE.max_downstream_for_method.remove(&method);

        let mut handler = RajomonHandler::new(method);
        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(20).build();
        let dropped = handler.check_inbound(&mut ctx);
        assert!(!dropped);
        assert_eq!(handler.remaining_tokens(), 13); // 20 - 7 = 13
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_mixed_tokens_accept_reject() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        RAJOMON_STATE.own_price.store(10, Ordering::Relaxed);
        let method = CowGrpcMethod::new("svc", "Foo_mixed");
        RAJOMON_STATE.max_downstream_for_method.remove(&method);

        let mut rejected = 0;
        let mut accepted = 0;
        for i in 0..10 {
            let token_val = if i < 5 { 3u64 } else { 20u64 };
            let mut handler = RajomonHandler::new(method.clone());
            let mut ctx = masa_core::ContextBuilder::new("test", 0)
                .tokens(token_val)
                .build();
            if handler.check_inbound(&mut ctx) {
                rejected += 1;
            } else {
                accepted += 1;
                if i >= 5 {
                    assert_eq!(handler.remaining_tokens(), 10); // 20 - 10
                }
            }
        }
        assert_eq!(rejected, 5);
        assert_eq!(accepted, 5);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_price_increases_then_decreases_over_time() {
        let state = RajomonSharedState::new();

        // 5 ticks of congestion (above threshold)
        for _ in 0..5 {
            state.queue_stats.window_max.store(20000, Ordering::Relaxed);
            state.update_prices();
        }
        assert_eq!(state.own_price.load(Ordering::Relaxed), 5 * PRICE_STEP_UP);

        // 3 ticks of no congestion
        for _ in 0..3 {
            state.update_prices(); // window_max already 0
        }
        assert_eq!(
            state.own_price.load(Ordering::Relaxed),
            5 * PRICE_STEP_UP - 3 * PRICE_STEP_DOWN
        );
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_global_own_price_shared_across_methods() {
        let state = RajomonSharedState::new();
        state.own_price.store(42, Ordering::Relaxed);
        let method_a = CowGrpcMethod::new("svc", "A");
        let method_b = CowGrpcMethod::new("svc", "B");

        let price_a = state.accumulated_price(&method_a);
        let price_b = state.accumulated_price(&method_b);
        assert_eq!(price_a, 42);
        assert_eq!(price_b, 42);
    }
}
