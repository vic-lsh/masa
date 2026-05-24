use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use once_cell::sync::Lazy;
use tonic_core::CowGrpcMethod;

use crate::policy_params::PolicyParams;

/// Global Rajomon state shared across all request handlers.
pub static RAJOMON_STATE: Lazy<RajomonSharedState> = Lazy::new(|| RajomonSharedState::new());

/// Global queue statistics for the current time window.
/// Tracks the maximum queue latency observed since the last tick.
pub struct QueueStats {
    /// Maximum per-request queue latency (us) observed since the last tick.
    pub window_max: AtomicU64,
    /// Maximum window_max seen across all ticks since last periodic log.
    /// Not reset by update_prices, only by log_pricing_tables.
    pub log_window_max: AtomicU64,
}

impl QueueStats {
    fn new() -> Self {
        Self {
            window_max: AtomicU64::new(0),
            log_window_max: AtomicU64::new(0),
        }
    }
}

impl std::fmt::Debug for QueueStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueueStats")
            .field("window_max", &self.window_max.load(Ordering::Relaxed))
            .field(
                "log_window_max",
                &self.log_window_max.load(Ordering::Relaxed),
            )
            .finish()
    }
}

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
    // ── Diagnostic counters (reset every log_pricing_tables call) ───
    /// Requests admitted at inbound check.
    pub diag_admitted: AtomicU64,
    /// Requests rejected at inbound check.
    pub diag_rejected: AtomicU64,
    /// Requests rejected at child-budget check.
    pub diag_child_budget_rej: AtomicU64,
    /// Price controller ticks where price increased.
    pub diag_price_up_ticks: AtomicU64,
    /// Price controller ticks where price decreased.
    pub diag_price_down_ticks: AtomicU64,
    /// Price controller ticks where price held (hysteresis band).
    pub diag_price_hold_ticks: AtomicU64,
    /// Sum of (accumulated_price - inbound_tokens) for rejected requests.
    pub diag_token_deficit_sum: AtomicU64,
}

impl RajomonSharedState {
    pub(crate) fn new() -> Self {
        Self {
            own_price: AtomicU64::new(PolicyParams::global().rajomon.init_price),
            downstream_prices: DashMap::new(),
            max_downstream_for_method: DashMap::new(),
            queue_stats: QueueStats::new(),
            diag_admitted: AtomicU64::new(0),
            diag_rejected: AtomicU64::new(0),
            diag_child_budget_rej: AtomicU64::new(0),
            diag_price_up_ticks: AtomicU64::new(0),
            diag_price_down_ticks: AtomicU64::new(0),
            diag_price_hold_ticks: AtomicU64::new(0),
            diag_token_deficit_sum: AtomicU64::new(0),
        }
    }

    /// Accumulated price = own_price + max(downstream prices).
    ///
    /// This matches the "Maximum Total Price" policy defined in the Rajomon
    /// paper (NSDI '25, §3.4): *"A service's price is the price of its local
    /// computation plus the maximum of prices published by relevant downstream
    /// services."* See `3rd_party/rajomon/RUST_PORT_ALIGNMENT.md` §0.2.
    ///
    /// NOTE: The Go reference implementation's `"maximal"` mode stores
    /// `max(ownPrice, downstreamPrice)` instead, which is strictly weaker than
    /// the paper's definition and causes false accepts at upper layers. We
    /// deliberately diverge from Go here to match the paper.
    pub fn accumulated_price(&self, method: &CowGrpcMethod) -> u64 {
        let own = self.own_price.load(Ordering::Relaxed);
        let downstream = self
            .max_downstream_for_method
            .get(method)
            .map(|v| *v)
            .unwrap_or(0);
        own.saturating_add(downstream)
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

    /// Paper (NSDI '25, §3.4 "Proportional Price Updates") price update law:
    ///
    /// - If queueing delay exceeds the threshold, increase price by an
    ///   increment proportional to the excess. The paper says "when queuing
    ///   delay exceeds the threshold by 1ms, the price increases by
    ///   approximately 3 to 13 tokens" — implemented here as
    ///   `(excess_us / 1000) * price_step_up`, so `price_step_up` is in the
    ///   paper's "tokens per 1ms excess" units.
    /// - If queueing delay is less than half the threshold, decrease price
    ///   by 1 token (paper hardcodes this).
    /// - Otherwise (hysteresis hold band), price is unchanged.
    ///
    /// §10.5 Option B: the `window_max` is not reset to 0 on read; instead
    /// it is halved, giving exponential decay with half-life of one tick.
    /// This keeps the queue signal alive during periods when admission
    /// rejection dries up the `before_poll` observation stream, preventing
    /// the self-defeating feedback loop described in RUST_PORT_ALIGNMENT.md §2.
    ///
    /// See 3rd_party/rajomon/RUST_PORT_ALIGNMENT.md §0.1 and §10.4/§10.5.
    pub(crate) fn update_prices(&self) {
        let p = &PolicyParams::global().rajomon;
        // §10.5 Option B: atomically halve the window_max and read its
        // previous value, so the signal persists for a few ticks even if no
        // new observations arrive. fetch_update is a CAS loop, so it safely
        // races with concurrent fetch_max calls from before_poll.
        let max_us = self
            .queue_stats
            .window_max
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| Some(v / 2))
            .unwrap_or(0);
        self.queue_stats
            .log_window_max
            .fetch_max(max_us, Ordering::Relaxed);
        let own = self.own_price.load(Ordering::Relaxed);
        let new_price = if max_us > p.latency_threshold_us {
            // Paper: increase proportionally to excess-over-threshold.
            // `price_step_up` is interpreted as "tokens per 1ms of excess."
            // The paper gives a typical range of 3-13 tokens per ms.
            let excess_us = max_us - p.latency_threshold_us;
            let increment = ((excess_us * p.price_step_up) / 1000).max(1);
            self.diag_price_up_ticks.fetch_add(1, Ordering::Relaxed);
            own.saturating_add(increment)
        } else if own > 0 && max_us < p.latency_threshold_us / 2 {
            // Paper: hardcoded -1 when below half-threshold.
            self.diag_price_down_ticks.fetch_add(1, Ordering::Relaxed);
            own.saturating_sub(1)
        } else {
            // Paper: hysteresis hold band [threshold/2, threshold].
            self.diag_price_hold_ticks.fetch_add(1, Ordering::Relaxed);
            own
        };
        self.own_price.store(new_price, Ordering::Relaxed);
        log::trace!(
            "Rajomon tick: max_us={} own={}->{} threshold={}",
            max_us,
            own,
            new_price,
            p.latency_threshold_us,
        );
    }

    /// Exponential decay on cached downstream prices.
    ///
    /// Each tick, every entry in `downstream_prices` is halved.  Entries
    /// that are continuously refreshed by incoming responses (lazy price
    /// propagation, paper §3.4) will stay near their true value — the
    /// fresh write overwrites the decayed value.  Entries that have gone
    /// stale (no responses arriving, e.g. because admission control has
    /// throttled the path) decay to zero in O(log₂(price)) ticks,
    /// breaking the stale-price deadlock where a high cached downstream
    /// price blocks all traffic and therefore prevents itself from being
    /// updated.
    ///
    /// This is analogous to the `window_max` halving in `update_prices`
    /// (§10.5 Option B) and replaces the old periodic full-wipe that
    /// caused a deterministic limit cycle (RUST_PORT_ALIGNMENT.md §6).
    pub(crate) fn decay_downstream_prices(&self) {
        let mut parents_to_update: Vec<CowGrpcMethod> = Vec::new();

        for mut entry in self.downstream_prices.iter_mut() {
            let old = *entry.value();
            if old > 0 {
                *entry.value_mut() = old / 2;
                let parent = entry.key().0.clone();
                if !parents_to_update.contains(&parent) {
                    parents_to_update.push(parent);
                }
            }
        }

        // Recompute max_downstream_for_method for affected parents.
        for parent in parents_to_update {
            let max_price = self
                .downstream_prices
                .iter()
                .filter(|e| e.key().0 == parent)
                .map(|e| *e.value())
                .max()
                .unwrap_or(0);
            self.max_downstream_for_method.insert(parent, max_price);
        }
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
        let peak = self.queue_stats.log_window_max.swap(0, Ordering::Relaxed);
        log::info!(
            "Rajomon queue_latency (peak_window_max over 5s): {} us",
            peak
        );
        // ── Diagnostic counters (500-tick window) ──
        let admitted = self.diag_admitted.swap(0, Ordering::Relaxed);
        let rejected = self.diag_rejected.swap(0, Ordering::Relaxed);
        let child_rej = self.diag_child_budget_rej.swap(0, Ordering::Relaxed);
        let up = self.diag_price_up_ticks.swap(0, Ordering::Relaxed);
        let down = self.diag_price_down_ticks.swap(0, Ordering::Relaxed);
        let hold = self.diag_price_hold_ticks.swap(0, Ordering::Relaxed);
        let deficit_sum = self.diag_token_deficit_sum.swap(0, Ordering::Relaxed);
        let mean_deficit = if rejected > 0 {
            deficit_sum / rejected
        } else {
            0
        };
        log::info!(
            "Rajomon diag: admitted={} rejected={} child_budget_rej={} \
             price_direction(up/down/hold)={}/{}/{} mean_token_deficit={}",
            admitted,
            rejected,
            child_rej,
            up,
            down,
            hold,
            mean_deficit,
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
                let mut interval = tokio::time::interval(Duration::from_millis(
                    PolicyParams::global().rajomon.price_update_rate_ms,
                ));
                let mut log_tick: u32 = 0;
                loop {
                    interval.tick().await;
                    RAJOMON_STATE.update_prices();
                    RAJOMON_STATE.decay_downstream_prices();
                    log_tick += 1;
                    // At 10ms tick rate, 500 ticks = 5s logging interval.
                    if log_tick >= 500 {
                        log_tick = 0;
                        RAJOMON_STATE.log_pricing_tables();
                    }
                }
            });
        }
    }
}
