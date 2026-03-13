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
            let new_price = if new_ewma > 1000 {
                // > 1ms average queue latency: scheduler is overloaded, raise price.
                current_price + 1
            } else {
                max(1, current_price.saturating_sub(1))
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
        let price = RAJOMON_STATE
            .local_prices
            .get(&self.rpc)
            .map(|v| *v)
            .unwrap_or(1);
        if !ctx.consume_tokens(price) {
            self.should_drop = true;
            true
        } else {
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
    pub(crate) fn check_outbound(
        &self,
        child_method: &CowGrpcMethod,
        ctx: &Context,
    ) -> Result<(), Status> {
        let price = RAJOMON_STATE
            .downstream_prices
            .get(child_method)
            .map(|v| *v)
            .unwrap_or(1);
        if ctx.tokens() < price {
            Err(self.issue_error(Some(child_method)))
        } else {
            Ok(())
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
        let price = RAJOMON_STATE
            .local_prices
            .get(&self.rpc)
            .map(|v| *v)
            .unwrap_or(1);
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
