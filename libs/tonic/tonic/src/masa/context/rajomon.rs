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
use std::cmp::max;
#[cfg(feature = "rajomon")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "rajomon")]
use std::time::Duration;

#[cfg(feature = "rajomon")]
const QUEUE_THRESHOLD_US: u64 = 1000;
#[cfg(feature = "rajomon")]
const PRICE_PER_EXCESS_MS: u64 = 10;
#[cfg(feature = "rajomon")]
const PRICE_DECREASE_STEP: u64 = 1;
#[cfg(feature = "rajomon")]
const PRICE_PROPAGATION_PROB: f64 = 0.2;

/// Global Rajomon state shared across all request handlers.
#[cfg(feature = "rajomon")]
pub static RAJOMON_STATE: Lazy<RajomonSharedState> = Lazy::new(|| RajomonSharedState::new());

/// Per-method queue statistics for the current time window and the running EWMA.
///
/// All fields are atomics so they can be updated from request handlers without holding any lock
/// beyond the DashMap shard lock used for the initial entry lookup.
#[cfg(feature = "rajomon")]
pub struct MethodQueueStats {
    /// Sum of per-request accumulated queue latencies (μs) collected since the last tick.
    window_sum: AtomicU64,
    /// Number of requests that completed since the last tick.
    window_count: AtomicU64,
    /// Time-based EWMA of the per-window average queue latency (μs).
    /// Updated once per tick by the background worker, independent of RPS.
    pub ewma_us: AtomicU64,
}

#[cfg(feature = "rajomon")]
impl MethodQueueStats {
    fn new() -> Self {
        Self {
            window_sum: AtomicU64::new(0),
            window_count: AtomicU64::new(0),
            ewma_us: AtomicU64::new(0),
        }
    }
}

#[cfg(feature = "rajomon")]
impl std::fmt::Debug for MethodQueueStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MethodQueueStats")
            .field("window_sum", &self.window_sum.load(Ordering::Relaxed))
            .field("window_count", &self.window_count.load(Ordering::Relaxed))
            .field("ewma_us", &self.ewma_us.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(feature = "rajomon")]
#[derive(Debug)]
#[allow(missing_docs)]
pub struct RajomonSharedState {
    pub local_prices: DashMap<CowGrpcMethod, u64>,
    pub downstream_prices: DashMap<CowGrpcMethod, u64>,
    pub queue_stats: DashMap<CowGrpcMethod, MethodQueueStats>,
}

#[cfg(feature = "rajomon")]
impl RajomonSharedState {
    fn new() -> Self {
        Self {
            local_prices: DashMap::new(),
            downstream_prices: DashMap::new(),
            queue_stats: DashMap::new(),
        }
    }

    /// Effective price = max(local, downstream) for a method.
    pub fn effective_price(&self, method: &CowGrpcMethod) -> u64 {
        let local = self.local_prices.get(method).map(|v| *v).unwrap_or(1);
        let downstream = self.downstream_prices.get(method).map(|v| *v).unwrap_or(0);
        std::cmp::max(local, downstream)
    }

    fn update_prices(&self) {
        for entry in self.queue_stats.iter() {
            // Atomically drain the current window's accumulated data and reset for the next tick.
            let sum = entry.window_sum.swap(0, Ordering::Relaxed);
            let count = entry.window_count.swap(0, Ordering::Relaxed);
            // If no requests arrived this window, window_avg = 0 and the EWMA decays toward 0,
            // correctly reflecting the absence of observed queueing pressure.
            let window_avg = if count > 0 { sum / count } else { 0 };

            // Time-based EWMA step: α = 1/4, half-life ≈ 2.4 ticks (240ms).
            // Decay is per-tick, not per-sample, so it is independent of RPS.
            // new_ewma = (1/4) * window_avg + (3/4) * old_ewma
            let old_ewma = entry.ewma_us.load(Ordering::Relaxed);
            let new_ewma = (window_avg + 3 * old_ewma) / 4;
            entry.ewma_us.store(new_ewma, Ordering::Relaxed);

            let method = entry.key();
            let current_price = self.local_prices.get(method).map(|v| *v).unwrap_or(1);
            let new_price = if new_ewma > QUEUE_THRESHOLD_US {
                // Proportional increase: more excess delay => larger price bump.
                let increment = ((new_ewma - QUEUE_THRESHOLD_US) / 1000 + 1) * PRICE_PER_EXCESS_MS;
                current_price + increment
            } else if new_ewma < QUEUE_THRESHOLD_US / 2 {
                // Well below threshold: slowly decrease price.
                max(1, current_price.saturating_sub(PRICE_DECREASE_STEP))
            } else {
                // Middle band: hold steady.
                current_price
            };
            self.local_prices.insert(method.clone(), new_price);
        }
    }

    fn log_pricing_tables(&self) {
        if !self.local_prices.is_empty() {
            let parts: Vec<String> = self
                .local_prices
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
            log::info!("Rajomon local_prices: {}", parts.join(", "));
        }
        if !self.downstream_prices.is_empty() {
            let parts: Vec<String> = self
                .downstream_prices
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
            log::info!("Rajomon downstream_prices: {}", parts.join(", "));
        }
        if !self.queue_stats.is_empty() {
            let parts: Vec<String> = self
                .queue_stats
                .iter()
                .map(|e| {
                    format!(
                        "{}::{}: {} us",
                        e.key().service(),
                        e.key().method(),
                        e.value().ewma_us.load(Ordering::Relaxed)
                    )
                })
                .collect();
            log::info!("Rajomon queue_latencies (ewma): {}", parts.join(", "));
        }
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
                let mut interval = tokio::time::interval(Duration::from_millis(100));
                let mut log_tick: u32 = 0;
                loop {
                    interval.tick().await;
                    RAJOMON_STATE.update_prices();
                    log_tick += 1;
                    if log_tick >= 50 {
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
    /// Accumulated scheduler queue latency across all polls for this request (microseconds).
    #[cfg(feature = "rajomon")]
    accumulated_q_lat_us: AtomicU64,
    /// Remaining token budget for this request, shared across fan-out branches.
    #[cfg(feature = "rajomon")]
    remaining_tokens: AtomicU64,
}

impl Default for RajomonHandler {
    fn default() -> Self {
        Self {
            #[cfg(feature = "rajomon")]
            rpc: CowGrpcMethod::new("", ""),
            #[cfg(feature = "rajomon")]
            should_drop: false,
            #[cfg(feature = "rajomon")]
            accumulated_q_lat_us: AtomicU64::new(0),
            #[cfg(feature = "rajomon")]
            remaining_tokens: AtomicU64::new(0),
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
                accumulated_q_lat_us: AtomicU64::new(0),
                remaining_tokens: AtomicU64::new(0),
            }
        }
        #[cfg(not(feature = "rajomon"))]
        {
            let _ = rpc;
            Self {}
        }
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn check_inbound(&mut self, ctx: &mut Context) -> bool {
        let price = RAJOMON_STATE.effective_price(&self.rpc);
        if !ctx.consume_tokens(price) {
            self.should_drop = true;
            true
        } else {
            self.remaining_tokens.store(ctx.tokens(), Ordering::Relaxed);
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
        let price = RAJOMON_STATE.effective_price(child_method);
        // CAS loop to atomically subtract price from remaining_tokens
        loop {
            let current = self.remaining_tokens.load(Ordering::Relaxed);
            if current < price {
                return Err(self.issue_error(Some(child_method)));
            }
            if self
                .remaining_tokens
                .compare_exchange_weak(
                    current,
                    current - price,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Ok(());
            }
        }
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

    /// Accumulate the scheduler queue latency for this poll. The window stats are updated once
    /// per request via `finalize_queue_delay`, so each request contributes one data point
    /// (its total accumulated scheduler queue wait time across all polls).
    #[cfg(feature = "rajomon")]
    pub(crate) fn track_queue_delay(&self) {
        let q_lat_us = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        self.accumulated_q_lat_us
            .fetch_add(q_lat_us, Ordering::Relaxed);
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn track_queue_delay(&self) {}

    /// Commit this request's total accumulated queue latency to the current time window.
    /// The background worker drains the window every 100ms and applies a time-based EWMA step,
    /// so the decay rate is independent of RPS. Call this exactly once per request.
    #[cfg(feature = "rajomon")]
    pub(crate) fn finalize_queue_delay(&self) {
        let total_us = self.accumulated_q_lat_us.load(Ordering::Relaxed);
        let entry = RAJOMON_STATE
            .queue_stats
            .entry(self.rpc.clone())
            .or_insert_with(MethodQueueStats::new);
        entry.window_sum.fetch_add(total_us, Ordering::Relaxed);
        entry.window_count.fetch_add(1, Ordering::Relaxed);
    }

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
                    RAJOMON_STATE
                        .downstream_prices
                        .insert(child_method.clone(), price);
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

    #[cfg(feature = "rajomon")]
    pub(crate) fn inject_price_to_response<T>(
        &self,
        result: &mut Result<crate::Response<T>, Status>,
    ) {
        // Lazy propagation: only send price on ~20% of responses
        if rand::random::<f64>() > PRICE_PROPAGATION_PROB {
            return;
        }
        let price = RAJOMON_STATE.effective_price(&self.rpc);
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
/// Each interface (method) has its own token pool. The client refuses to send
/// requests when the pool for that interface is insufficient.
#[cfg(feature = "rajomon")]
#[allow(missing_debug_implementations)]
pub struct ClientTokenBucket {
    pools: DashMap<CowGrpcMethod, AtomicU64>,
    cached_prices: DashMap<CowGrpcMethod, u64>,
    replenish_amount: u64,
    max_tokens: u64,
}

#[cfg(feature = "rajomon")]
impl ClientTokenBucket {
    fn new() -> Self {
        Self {
            pools: DashMap::new(),
            cached_prices: DashMap::new(),
            replenish_amount: 100,
            max_tokens: 1000,
        }
    }

    /// Try to acquire tokens for a method. Returns the number of tokens to assign
    /// to the request if successful, or None if the pool is insufficient.
    pub fn try_acquire(&self, method: &CowGrpcMethod) -> Option<u64> {
        let price = self.cached_prices.get(method).map(|v| *v).unwrap_or(1);
        let pool = self
            .pools
            .entry(method.clone())
            .or_insert_with(|| AtomicU64::new(self.max_tokens));

        // CAS loop to atomically subtract price from pool
        loop {
            let current = pool.value().load(Ordering::Relaxed);
            if current < price {
                return None;
            }
            if pool
                .value()
                .compare_exchange_weak(
                    current,
                    current - price,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                // Assign random tokens to the request (1..=100)
                let tokens = rand::Rng::gen_range(&mut rand::thread_rng(), 1..=100u64);
                return Some(tokens);
            }
        }
    }

    /// Update the cached price for a method (called when response header received).
    pub fn update_price(&self, method: &CowGrpcMethod, price: u64) {
        self.cached_prices.insert(method.clone(), price);
    }

    /// Replenish all pools by `replenish_amount`, capped at `max_tokens`.
    pub fn replenish(&self) {
        for entry in self.pools.iter() {
            let current = entry.value().load(Ordering::Relaxed);
            let new_val = std::cmp::min(current + self.replenish_amount, self.max_tokens);
            entry.value().store(new_val, Ordering::Relaxed);
        }
    }

    /// Start the background replenishment worker (10ms interval).
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
                let mut interval = tokio::time::interval(Duration::from_millis(10));
                loop {
                    interval.tick().await;
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

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_proportional_price_increase() {
        let state = RajomonSharedState::new();
        let method = CowGrpcMethod::new("svc", "method");

        // Simulate a window with high queue latency (5000us avg)
        state
            .queue_stats
            .entry(method.clone())
            .or_insert_with(MethodQueueStats::new);
        let stats = state.queue_stats.get(&method).unwrap();
        stats.window_sum.store(50000, Ordering::Relaxed);
        stats.window_count.store(10, Ordering::Relaxed);

        state.update_prices();

        let price = state.local_prices.get(&method).map(|v| *v).unwrap_or(1);
        // EWMA = (5000 + 0) / 4 = 1250, excess = 250, increment = (250/1000+1)*10 = 10
        // new_price = 1 (default) + 10 = 11
        assert_eq!(price, 11);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_price_middle_band_holds_steady() {
        let state = RajomonSharedState::new();
        let method = CowGrpcMethod::new("svc", "method");

        // Set initial price to 5
        state.local_prices.insert(method.clone(), 5);
        state
            .queue_stats
            .entry(method.clone())
            .or_insert_with(MethodQueueStats::new);

        // Simulate EWMA in middle band (600us: between 500 and 1000)
        let stats = state.queue_stats.get(&method).unwrap();
        // We need window_avg = 2400 so EWMA = (2400 + 0)/4 = 600
        stats.window_sum.store(24000, Ordering::Relaxed);
        stats.window_count.store(10, Ordering::Relaxed);

        state.update_prices();

        let price = state.local_prices.get(&method).map(|v| *v).unwrap_or(1);
        assert_eq!(price, 5); // unchanged
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_price_decrease_below_half_threshold() {
        let state = RajomonSharedState::new();
        let method = CowGrpcMethod::new("svc", "method");

        // Set initial price to 5
        state.local_prices.insert(method.clone(), 5);
        state
            .queue_stats
            .entry(method.clone())
            .or_insert_with(MethodQueueStats::new);

        // Simulate very low EWMA (100us: below 500)
        let stats = state.queue_stats.get(&method).unwrap();
        stats.window_sum.store(1000, Ordering::Relaxed);
        stats.window_count.store(10, Ordering::Relaxed);

        state.update_prices();

        let price = state.local_prices.get(&method).map(|v| *v).unwrap_or(1);
        // EWMA = 100/4 = 25, below threshold/2, decrease by 1: 5 -> 4
        assert_eq!(price, 4);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_price_floor_at_one() {
        let state = RajomonSharedState::new();
        let method = CowGrpcMethod::new("svc", "method");

        state.local_prices.insert(method.clone(), 1);
        state
            .queue_stats
            .entry(method.clone())
            .or_insert_with(MethodQueueStats::new);

        state.update_prices();

        let price = state.local_prices.get(&method).map(|v| *v).unwrap_or(1);
        assert_eq!(price, 1); // can't go below 1
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_effective_price_max_of_local_downstream() {
        let state = RajomonSharedState::new();
        let method = CowGrpcMethod::new("svc", "method");

        // No prices set: default local=1, downstream=0 -> effective=1
        assert_eq!(state.effective_price(&method), 1);

        // Local=5, no downstream -> effective=5
        state.local_prices.insert(method.clone(), 5);
        assert_eq!(state.effective_price(&method), 5);

        // Local=5, downstream=3 -> effective=5
        state.downstream_prices.insert(method.clone(), 3);
        assert_eq!(state.effective_price(&method), 5);

        // Local=5, downstream=10 -> effective=10
        state.downstream_prices.insert(method.clone(), 10);
        assert_eq!(state.effective_price(&method), 10);
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_client_token_bucket_acquire_and_replenish() {
        let bucket = ClientTokenBucket::new();
        let method = CowGrpcMethod::new("svc", "method");

        // First acquire should succeed (pool starts at max_tokens=1000, price defaults to 1)
        let result = bucket.try_acquire(&method);
        assert!(result.is_some());
        let tokens = result.unwrap();
        assert!(tokens >= 1 && tokens <= 100);

        // Pool should now be 999
        let pool_val = bucket
            .pools
            .get(&method)
            .unwrap()
            .value()
            .load(Ordering::Relaxed);
        assert_eq!(pool_val, 999);

        // Set a high price
        bucket.update_price(&method, 2000);

        // Acquire should fail (pool=999 < price=2000)
        let result = bucket.try_acquire(&method);
        assert!(result.is_none());

        // Replenish should add 100, pool = 999 + 100 = 1000 (capped at max)
        bucket.replenish();
        let pool_val = bucket
            .pools
            .get(&method)
            .unwrap()
            .value()
            .load(Ordering::Relaxed);
        assert_eq!(pool_val, 1000);

        // Still can't acquire at price 2000
        assert!(bucket.try_acquire(&method).is_none());
    }

    #[cfg(feature = "rajomon")]
    #[test]
    fn test_remaining_tokens_after_check_inbound() {
        let method = CowGrpcMethod::new("svc", "method");
        let mut handler = RajomonHandler::new(method);

        let mut ctx = masa_core::ContextBuilder::new("test", 0)
            .tokens(50)
            .build();

        // Default price is 1, so 50-1=49 remaining
        let dropped = handler.check_inbound(&mut ctx);
        assert!(!dropped);
        assert_eq!(handler.remaining_tokens(), 49);
    }
}
