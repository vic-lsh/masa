// Rajomon token-based admission control layer.
//
// Implements per-request token-budget admission control with server-side
// price signals and client-side token bucket rate limiting. Aligned with
// the original Go implementation (3rd_party/rajomon/).

use std::task::Poll;
use std::time::Duration;

use dashmap::DashMap;
use masa_core::Context;
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU64, Ordering};
use tonic_core::{CowGrpcMethod, Response, Status};

use super::super::{ChildRpcContext, Layer, LayerChild, LayerServer};
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
            own.saturating_add(increment).min(p.price_cap)
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
    fn ensure_worker_started() {
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

// ── Layer Implementation ────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) struct RajomonServer;

impl LayerServer for RajomonServer {
    fn new() -> Self {
        Self
    }
}

#[derive(Debug)]
pub(crate) struct RajomonLayer {
    rpc: CowGrpcMethod,
    should_drop: bool,
    /// Remaining token budget for this request, shared across fan-out branches.
    remaining_tokens: AtomicU64,
    /// Inbound token count from the request context (for deterministic price propagation).
    inbound_tokens: AtomicU64,
}

impl Layer for RajomonLayer {
    type Server = RajomonServer;
    type Child = RajomonChild;

    fn new(method: &CowGrpcMethod, _server: &RajomonServer, ctx: &mut Context) -> Self {
        RajomonSharedState::ensure_worker_started();

        let mut layer = Self {
            rpc: method.clone(),
            should_drop: false,
            remaining_tokens: AtomicU64::new(0),
            inbound_tokens: AtomicU64::new(0),
        };

        // Inbound admission check: accepted iff ctx.tokens() >= accumulated_price.
        // Paper §3.4 / Go LoadShedding both allow ctx.tokens == accumulated == 0
        // to pass (nothing to charge for). No artificial minimum price.
        let accumulated = RAJOMON_STATE.accumulated_price(&layer.rpc);
        let own = RAJOMON_STATE.own_price.load(Ordering::Relaxed);
        layer.inbound_tokens.store(ctx.tokens(), Ordering::Relaxed);
        if ctx.tokens() < accumulated {
            layer.should_drop = true;
            RAJOMON_STATE.diag_rejected.fetch_add(1, Ordering::Relaxed);
            RAJOMON_STATE
                .diag_token_deficit_sum
                .fetch_add(accumulated - ctx.tokens(), Ordering::Relaxed);
        } else {
            layer
                .remaining_tokens
                .store(ctx.tokens() - own, Ordering::Relaxed);
            RAJOMON_STATE.diag_admitted.fetch_add(1, Ordering::Relaxed);
        }

        layer
    }

    #[inline]
    fn before_poll<Ret>(&self, _ctx: &Context) -> Result<(), Result<Response<Ret>, Status>> {
        // Track queue delay for price updates
        let q_lat_us = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        RAJOMON_STATE
            .queue_stats
            .window_max
            .fetch_max(q_lat_us, Ordering::Relaxed);

        // Check if request was marked for drop
        if self.should_drop {
            return Err(Err(self.issue_error(None, "RajomonAdmissionRej")));
        }
        Ok(())
    }

    #[inline]
    fn before_child_rpc<T>(
        &self,
        _ctx: &Context,
        child_method: &CowGrpcMethod,
        _child_ctx: &mut RajomonChild,
        _request: &mut tonic_core::Request<T>,
        child_rpc: &mut ChildRpcContext,
    ) -> Result<(), Status> {
        // Check if request was marked for drop before initiating child RPC
        if self.should_drop {
            return Err(self.issue_error(None, "RajomonAdmissionRej"));
        }

        // Check outbound budget
        let price = RAJOMON_STATE.child_price(child_method);
        let current = self.remaining_tokens.load(Ordering::Relaxed);
        if current < price {
            RAJOMON_STATE
                .diag_child_budget_rej
                .fetch_add(1, Ordering::Relaxed);
            return Err(self.issue_error(Some(child_method), "RajomonChildBudgetRej"));
        }

        child_rpc.tokens = self.remaining_tokens.load(Ordering::Relaxed);

        Ok(())
    }

    #[inline]
    fn after_poll<Ret>(
        &self,
        _ctx: &Context,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        if let Poll::Pending = poll {
            if self.should_drop {
                return Err(Err(self.issue_error(None, "RajomonAdmissionRej")));
            }
        }
        Ok(())
    }

    #[inline]
    fn after_child_rpc<T>(
        &self,
        _ctx: &Context,
        child_method: &CowGrpcMethod,
        response: &mut Result<Response<T>, Status>,
        _child_ctx: &RajomonChild,
    ) -> Result<(), Status> {
        // Extract and cache downstream prices from child response
        let metadata = match response {
            Ok(resp) => resp.metadata(),
            Err(status) => status.metadata(),
        };

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

        Ok(())
    }

    #[inline]
    fn finalize<Ret>(&self, _ctx: &mut Context, result: &mut Result<Response<Ret>, Status>) {
        // Paper §3.4 "Lazy Price Propagation": probabilistic per-response.
        if !self.should_propagate_price() {
            return;
        }
        // Paper §3.4: propagate the raw accumulated price — no artificial floor.
        let price = RAJOMON_STATE.accumulated_price(&self.rpc);
        if let Ok(value) = tonic_core::metadata::MetadataValue::try_from(price.to_string()) {
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
}

impl RajomonLayer {
    /// Build a rejection `Status` carrying a structured `/EarlyReturn?...` message.
    ///
    /// Mirrors the format used by `predictive.rs` so the experiment plotting code
    /// (`exp_runner/runner/plotting/util.py::_parse_error_columns`) can extract a
    /// `reason` column for each rejected request.
    fn issue_error(&self, child_method: Option<&CowGrpcMethod>, reason: &str) -> Status {
        let msg = match child_method {
            Some(child) => format!(
                "/EarlyReturn?src={}::{}?last_rpc={}::{}&reason={}",
                self.rpc.service(),
                self.rpc.method(),
                child.service(),
                child.method(),
                reason,
            ),
            None => format!(
                "/EarlyReturn?src={}::{}&reason={}",
                self.rpc.service(),
                self.rpc.method(),
                reason,
            ),
        };

        Status::resource_exhausted(msg)
    }

    /// Paper §3.4 "Lazy Price Propagation": *"Upon sending a response, the
    /// controller attaches its price information to the response with a
    /// configured probability, e.g., 20%, updating the upstream services
    /// with the current local prices."*
    ///
    /// `price_freq` is interpreted as the inverse propagation probability:
    /// `1/price_freq` is the probability of attaching the price on each
    /// response. So `price_freq = 5` matches the paper's 20% example,
    /// `price_freq = 1` means always propagate, and `price_freq = 0` means
    /// never propagate.
    ///
    /// This replaces the prior `inbound_tokens % price_freq == 0` predicate
    /// inherited from the Go reference (`3rd_party/rajomon/rajomon.go:415` /
    /// `:442`). The Go predicate is biased — it depends on the distribution
    /// of `tok` values across requests, and under deterministic "all-in"
    /// client spending it degenerates to "always" or "never." See
    /// `3rd_party/rajomon/RUST_PORT_ALIGNMENT.md` §0.5 / §10 item B.
    fn should_propagate_price(&self) -> bool {
        use rand::Rng;
        let p = PolicyParams::global().rajomon.price_freq;
        match p {
            0 => false,
            1 => true,
            n => rand::thread_rng().gen_range(0..n) == 0,
        }
    }
}

// ── Per-Child-RPC ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct RajomonChild;

impl LayerChild for RajomonChild {
    fn new() -> Self {
        Self
    }
}

// ── Client Token Bucket ─────────────────────────────────────────────────

/// Client-side token bucket for Rajomon rate limiting.
/// Single global counter matching the original Go implementation.
#[allow(missing_debug_implementations)]
pub struct ClientTokenBucket {
    tokens_left: AtomicU64,
    cached_prices: DashMap<CowGrpcMethod, u64>,
}

impl ClientTokenBucket {
    fn new() -> Self {
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

    /// Replenish token pool by `token_update_step`, capped at `max_token`.
    pub fn replenish(&self) {
        let p = &PolicyParams::global().rajomon;
        loop {
            let current = self.tokens_left.load(Ordering::Relaxed);
            let new_val = (current + p.token_update_step).min(p.max_token);
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

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Default parameter values matching PolicyParams defaults.
    // Tests verify algorithmic behaviour with these specific values.
    // NOTE: `price_step_down` is intentionally not referenced — under the
    // paper's algorithm decay is hardcoded to -1 per tick (see §10.4 in
    // 3rd_party/rajomon/RUST_PORT_ALIGNMENT.md).
    const LATENCY_THRESHOLD_US: u64 = 5_000;
    const INIT_PRICE: u64 = 0;
    const PRICE_STEP_UP: u64 = 8;
    const PRICE_CAP: u64 = u64::MAX;
    const TOKENS_LEFT_INIT: u64 = 10;
    const TOKEN_UPDATE_STEP: u64 = 5;
    const MAX_TOKEN: u64 = 100;

    /// Mutex to serialize tests that modify the global RAJOMON_STATE.own_price,
    /// since it's a single global value shared across all test threads.
    static GLOBAL_STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ── A. Price Update Algorithm Tests (Step Strategy) ──

    /// Paper §3.4: when queueing exceeds threshold by 1ms, the price increases
    /// by approximately `price_step_up` tokens (the paper reports 3-13).
    /// Our `update_prices` formula is `((excess_us * step_up) / 1000).max(1)`.
    #[test]
    fn test_proportional_price_increase_on_congestion() {
        let state = RajomonSharedState::new();
        // Set window_max to 1ms over threshold so the proportional formula
        // yields exactly PRICE_STEP_UP tokens of increment per tick.
        state
            .queue_stats
            .window_max
            .store(LATENCY_THRESHOLD_US + 1000, Ordering::Relaxed);
        state.update_prices();
        assert_eq!(
            state.own_price.load(Ordering::Relaxed),
            INIT_PRICE + PRICE_STEP_UP
        );

        // Second tick, still congested. Need to re-set window_max because the
        // §10.5 halving means it doesn't carry forward at full magnitude.
        state
            .queue_stats
            .window_max
            .store(LATENCY_THRESHOLD_US + 1000, Ordering::Relaxed);
        state.update_prices();
        assert_eq!(
            state.own_price.load(Ordering::Relaxed),
            INIT_PRICE + 2 * PRICE_STEP_UP
        );
    }

    /// Paper §3.4: when queueing is below half the threshold, the price
    /// decreases by exactly 1 token per tick (hardcoded). The configurable
    /// `price_step_down` parameter is unused under the paper's algorithm.
    #[test]
    fn test_paper_price_decrease_is_one_per_tick() {
        let state = RajomonSharedState::new();
        state.own_price.store(5, Ordering::Relaxed);
        state.update_prices();
        assert_eq!(state.own_price.load(Ordering::Relaxed), 4);
        state.update_prices();
        assert_eq!(state.own_price.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn test_price_floor_at_zero() {
        let state = RajomonSharedState::new();
        state.own_price.store(1, Ordering::Relaxed);
        state.update_prices();
        assert_eq!(state.own_price.load(Ordering::Relaxed), 0);
        state.update_prices();
        assert_eq!(state.own_price.load(Ordering::Relaxed), 0);
    }

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

    /// Paper §3.4: price increase is *proportional* to queuing-delay excess.
    /// Severe congestion should produce a larger increment than mild
    /// congestion. Replaces the previous `test_price_increase_uses_constant_
    /// step_not_proportional` test, which encoded the pre-paper step semantics.
    #[test]
    fn test_price_increase_is_proportional_to_excess() {
        let state = RajomonSharedState::new();

        // Mild congestion: 1ms over threshold → exactly PRICE_STEP_UP increment.
        state.own_price.store(0, Ordering::Relaxed);
        state
            .queue_stats
            .window_max
            .store(LATENCY_THRESHOLD_US + 1000, Ordering::Relaxed);
        state.update_prices();
        let price_after_mild = state.own_price.load(Ordering::Relaxed);

        // Severe congestion: 10ms over threshold → 10× the increment.
        // (10000us excess * 8 step_up) / 1000 = 80.
        state.own_price.store(0, Ordering::Relaxed);
        state
            .queue_stats
            .window_max
            .store(LATENCY_THRESHOLD_US + 10_000, Ordering::Relaxed);
        state.update_prices();
        let price_after_severe = state.own_price.load(Ordering::Relaxed);

        assert_eq!(price_after_mild, PRICE_STEP_UP);
        // Severe should be much larger than mild — proportionality.
        assert!(price_after_severe > price_after_mild);
        // 10ms excess × 8 tokens/ms = 80
        assert_eq!(price_after_severe, 80);
    }

    // ── B. Queue Delay Signal Tests (Window Max) ──

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

    /// §10.5 Option B: window_max is *halved* on read, not reset to 0. This
    /// keeps the queue-latency signal alive when admission rejection dries up
    /// the per-request observation stream from `before_poll`. See
    /// `RUST_PORT_ALIGNMENT.md` §0 / §10.5 for the rationale.
    #[test]
    fn test_window_max_halves_after_price_update() {
        let state = RajomonSharedState::new();
        state.queue_stats.window_max.store(5000, Ordering::Relaxed);
        state.update_prices();
        // Halved, not reset to zero.
        assert_eq!(state.queue_stats.window_max.load(Ordering::Relaxed), 2500);
        state.update_prices();
        assert_eq!(state.queue_stats.window_max.load(Ordering::Relaxed), 1250);
    }

    // ── C. Price Aggregation Tests (Paper "Maximum Total Price") ──

    /// Paper §3.4: `total_price = own + max(downstream prices)`.
    /// See 3rd_party/rajomon/RUST_PORT_ALIGNMENT.md §0.2 for why this differs
    /// from the Go reference implementation's `max(own, downstream)` form.
    #[test]
    fn test_accumulated_price_is_own_plus_max_downstream() {
        let state = RajomonSharedState::new();
        let method = CowGrpcMethod::new("svc", "method");

        // own=5, downstream=3 -> 5 + 3 = 8
        state.own_price.store(5, Ordering::Relaxed);
        state.max_downstream_for_method.insert(method.clone(), 3);
        assert_eq!(state.accumulated_price(&method), 8);

        // own=5, downstream=10 -> 5 + 10 = 15
        state.max_downstream_for_method.insert(method.clone(), 10);
        assert_eq!(state.accumulated_price(&method), 15);

        // own=0, no downstream -> 0 + 0 = 0
        state.own_price.store(0, Ordering::Relaxed);
        let method2 = CowGrpcMethod::new("svc", "other");
        assert_eq!(state.accumulated_price(&method2), 0);
    }

    #[test]
    fn test_accumulated_price_own_only_no_downstream() {
        let state = RajomonSharedState::new();
        state.own_price.store(7, Ordering::Relaxed);
        let method = CowGrpcMethod::new("svc", "method");
        assert_eq!(state.accumulated_price(&method), 7);
    }

    // ── D. Token Consumption Tests (Deduction at Each Hop) ──

    #[test]
    fn test_check_inbound_deducts_own_price_not_accumulated() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        let method = CowGrpcMethod::new("svc", "method");
        RAJOMON_STATE.own_price.store(3, Ordering::Relaxed);
        // Clear any downstream for this method
        RAJOMON_STATE
            .max_downstream_for_method
            .remove(&CowGrpcMethod::new("svc", "method"));

        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(20).build();
        let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
        assert!(!layer.should_drop);
        assert_eq!(layer.remaining_tokens.load(Ordering::Relaxed), 17); // 20 - own(3) = 17
    }

    /// The inbound gate uses the paper's `accumulated = own + max(downstream)`
    /// price for the admission check, but deducts only `own_price` from the
    /// forwarded token budget — downstream children will then be charged
    /// against their own prices via the cascaded token budget.
    #[test]
    fn test_check_inbound_deducts_own_not_accumulated_when_downstream_dominant() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        let method = CowGrpcMethod::new("svc", "method_downstream_dom");
        RAJOMON_STATE.own_price.store(5, Ordering::Relaxed);
        RAJOMON_STATE
            .max_downstream_for_method
            .insert(method.clone(), 20); // accumulated = 5 + 20 = 25
        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(25).build();
        let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
        assert!(!layer.should_drop); // tok(25) >= accumulated(25) -> admitted
        assert_eq!(layer.remaining_tokens.load(Ordering::Relaxed), 20); // 25 - own(5) = 20
    }

    #[test]
    fn test_check_inbound_rejects_insufficient_tokens() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        let method = CowGrpcMethod::new("svc", "reject_method");
        RAJOMON_STATE.own_price.store(100, Ordering::Relaxed);

        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(10).build();
        let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
        assert!(layer.should_drop);
    }

    #[test]
    fn test_check_inbound_accepts_exact_tokens() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        let method = CowGrpcMethod::new("svc", "exact_method");
        RAJOMON_STATE.own_price.store(10, Ordering::Relaxed);
        RAJOMON_STATE
            .max_downstream_for_method
            .remove(&CowGrpcMethod::new("svc", "exact_method"));

        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(10).build();
        let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
        assert!(!layer.should_drop);
        assert_eq!(layer.remaining_tokens.load(Ordering::Relaxed), 0); // 10 - 10 = 0
    }

    #[test]
    fn test_check_inbound_tokens_zero_price_zero() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        let method = CowGrpcMethod::new("svc", "zero_method");
        RAJOMON_STATE.own_price.store(0, Ordering::Relaxed);
        RAJOMON_STATE
            .max_downstream_for_method
            .remove(&CowGrpcMethod::new("svc", "zero_method"));

        // Paper §3.4 / Go LoadShedding: accumulated = 0 and tokens = 0 means
        // there is nothing to charge the request for, so it is admitted.
        // This test used to assert rejection under our now-removed .max(1)
        // minimum-effective-price clamp (§10.7).
        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(0).build();
        let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
        assert!(!layer.should_drop);
    }

    // ── E. Downstream Price Tests (Max Recomputation, Not Ratchet) ──

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

    #[test]
    fn test_child_price_default_zero() {
        let state = RajomonSharedState::new();
        let method = CowGrpcMethod::new("svc", "method");
        assert_eq!(state.child_price(&method), 0);
    }

    // ── F. Client Token Bucket Tests (Single Global Counter) ──

    /// `try_acquire` returns a uniform-random token count in `[0, current]`
    /// per the paper's Randomized Token Spending policy (§3.3). The exact
    /// returned value is non-deterministic, so we only assert it's in range.
    #[test]
    fn test_client_token_bucket_single_global_counter() {
        let bucket = ClientTokenBucket::new();
        let method_a = CowGrpcMethod::new("svc", "A");
        let method_b = CowGrpcMethod::new("svc", "B");

        // Bucket starts at TOKENS_LEFT_INIT.
        assert_eq!(bucket.tokens_left(), TOKENS_LEFT_INIT);

        // First acquire returns a value in [0, TOKENS_LEFT_INIT].
        let tok_a = bucket.try_acquire(&method_a).expect("should succeed");
        assert!(tok_a <= TOKENS_LEFT_INIT);

        // Bucket should have TOKENS_LEFT_INIT - tok_a left, so subsequent
        // acquires can also succeed (cached_price defaults to 0).
        let tok_b = bucket.try_acquire(&method_b);
        assert!(tok_b.is_some());
    }

    /// Paper §3.3: token spending is uniform random in `[0, current]`. The
    /// returned value should fall within the bucket balance and not equal the
    /// balance with any deterministic regularity. We probe the distribution
    /// with multiple draws and assert at least one falls strictly below the
    /// max — a deterministic "all" strategy would always return `current`.
    #[test]
    fn test_client_token_spending_is_randomized() {
        let bucket = ClientTokenBucket::new();
        // Refill to MAX_TOKEN so each draw has a wide [0, MAX_TOKEN] range.
        for _ in 0..((MAX_TOKEN / TOKEN_UPDATE_STEP) + 2) {
            bucket.replenish();
        }
        assert_eq!(bucket.tokens_left(), MAX_TOKEN);

        // After 50 draws against a high bucket level (replenished each time),
        // a uniform-random strategy will produce values strictly below MAX
        // with overwhelming probability. The deterministic "all" strategy
        // would never produce such a value.
        let mut saw_strict_below = false;
        let method = CowGrpcMethod::new("svc", "m");
        for _ in 0..50 {
            // Top up the bucket between draws so the balance stays near MAX.
            for _ in 0..((MAX_TOKEN / TOKEN_UPDATE_STEP) + 2) {
                bucket.replenish();
            }
            let tok = bucket.try_acquire(&method).expect("should succeed");
            assert!(tok <= MAX_TOKEN);
            if tok < MAX_TOKEN {
                saw_strict_below = true;
            }
        }
        assert!(
            saw_strict_below,
            "uniform-random spending should produce at least one tok < MAX_TOKEN over 50 draws"
        );
    }

    /// `replenish` saturates at `max_token`. After enough refills, the bucket
    /// balance equals `MAX_TOKEN` (independent of randomized spending).
    #[test]
    fn test_client_replenish_caps_at_max() {
        let bucket = ClientTokenBucket::new();
        // Replenish enough times to reach MAX_TOKEN from TOKENS_LEFT_INIT
        // and exceed it (to verify the cap clamps it back to MAX_TOKEN).
        for _ in 0..((MAX_TOKEN - TOKENS_LEFT_INIT) / TOKEN_UPDATE_STEP + 5) {
            bucket.replenish();
        }
        assert_eq!(bucket.tokens_left(), MAX_TOKEN);
    }

    #[test]
    fn test_client_rate_limits_when_insufficient() {
        let bucket = ClientTokenBucket::new();
        let method = CowGrpcMethod::new("svc", "method");
        bucket.update_price(&method, MAX_TOKEN + 1);

        let result = bucket.try_acquire(&method);
        assert!(result.is_none());
    }

    // ── G. Price Propagation Tests (Probabilistic, paper §3.4) ──

    /// Paper §3.4: probabilistic propagation with `1/price_freq` rate.
    /// `price_freq = 1` should always propagate; `price_freq = 0` should
    /// never propagate. These two edges are deterministic and easy to assert.
    #[test]
    fn test_price_propagation_edge_cases() {
        let method = CowGrpcMethod::new("svc", "m");
        let mut ctx = masa_core::ContextBuilder::new("test", 0)
            .tokens(100)
            .build();
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        RAJOMON_STATE.own_price.store(0, Ordering::Relaxed);
        RAJOMON_STATE.max_downstream_for_method.remove(&method);
        let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);

        // We can't override PolicyParams::global() at runtime here, so we
        // can't easily test arbitrary price_freq values from the test
        // environment. The compile-time default is `price_freq = 5` (see
        // RajomonParams::default), which gives a ~20% propagation rate.
        // Verify the predicate at least returns *some* mixture of true and
        // false over many calls.
        let n_trials = 5000;
        let mut true_count = 0;
        for _ in 0..n_trials {
            if layer.should_propagate_price() {
                true_count += 1;
            }
        }
        // With price_freq=5, expected rate is ~20% = 1000 out of 5000.
        // With p=0.2, n=5000, std-dev = sqrt(5000 * 0.2 * 0.8) ≈ 28.
        // 4σ tolerance: |observed - 1000| < 112. Use 200 for headroom.
        assert!(
            (800..=1200).contains(&true_count),
            "expected ~20% propagation rate (1000±200), got {} / {}",
            true_count,
            n_trials
        );
    }

    // ── H. End-to-End Algorithmic Equivalence Tests ──

    #[test]
    fn test_load_shedding_returns_correct_remaining_tokens() {
        let _lock = GLOBAL_STATE_LOCK.lock().unwrap();
        RAJOMON_STATE.own_price.store(7, Ordering::Relaxed);
        let method = CowGrpcMethod::new("svc", "Foo_ls");
        RAJOMON_STATE.max_downstream_for_method.remove(&method);

        let mut ctx = masa_core::ContextBuilder::new("test", 0).tokens(20).build();
        let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
        assert!(!layer.should_drop);
        assert_eq!(layer.remaining_tokens.load(Ordering::Relaxed), 13); // 20 - 7 = 13
    }

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
            let mut ctx = masa_core::ContextBuilder::new("test", 0)
                .tokens(token_val)
                .build();
            let layer = RajomonLayer::new(&method, &RajomonServer, &mut ctx);
            if layer.should_drop {
                rejected += 1;
            } else {
                accepted += 1;
                if i >= 5 {
                    assert_eq!(layer.remaining_tokens.load(Ordering::Relaxed), 10);
                    // 20 - 10
                }
            }
        }
        assert_eq!(rejected, 5);
        assert_eq!(accepted, 5);
    }

    #[test]
    fn test_price_increases_then_decreases_over_time() {
        let state = RajomonSharedState::new();

        // 5 ticks of congestion at 1ms over the threshold → exactly
        // PRICE_STEP_UP increment per tick under the proportional formula.
        for _ in 0..5 {
            state
                .queue_stats
                .window_max
                .store(LATENCY_THRESHOLD_US + 1000, Ordering::Relaxed);
            state.update_prices();
        }
        assert_eq!(state.own_price.load(Ordering::Relaxed), 5 * PRICE_STEP_UP);

        // Wipe the persistent halved signal so the decay branch fires
        // immediately on the no-congestion ticks.
        state.queue_stats.window_max.store(0, Ordering::Relaxed);

        // 3 ticks of no congestion → -1 per tick (paper-hardcoded decay).
        for _ in 0..3 {
            state.update_prices();
        }
        assert_eq!(
            state.own_price.load(Ordering::Relaxed),
            5 * PRICE_STEP_UP - 3
        );
    }

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
