// Predictive admission control layer — latency estimation, deadline
// tightening, and predictive admission control.
//
// When `estimator` is enabled, this layer tracks latency distributions,
// tightens child deadlines (when `sched_pred` is also enabled), and runs
// predictive admission control.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Poll;
use std::time::Instant;

use masa_core::{Context, PriorityHint};
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::super::{ChildRpcContext, Layer, LayerChild, LayerServer};
use crate::layer::est::estimator::DefaultLatencyEstimator;
use crate::layer::est::state::{
    is_early_return_response, EstChildState, EstRequestState, EstServerState,
};
use crate::MethodRegistry;

// ── Server ──────────────────────────────────────────────────────────────

/// Server-level predictive layer state (shared across requests).
#[derive(Debug)]
pub(crate) struct PredAdmissionServer {
    est: Arc<EstServerState<DefaultLatencyEstimator>>,
    pred_admission: Arc<PredictiveAdmission>,
}

impl LayerServer for PredAdmissionServer {
    fn new() -> Self {
        Self {
            est: Arc::new(EstServerState::new()),
            pred_admission: Arc::new(PredictiveAdmission::new()),
        }
    }
}

// ── Per-Request ─────────────────────────────────────────────────────────

/// Per-request predictive layer state.
#[derive(Debug)]
pub(crate) struct PredAdmissionLayer {
    pub(crate) est: EstRequestState<DefaultLatencyEstimator>,
    pred_admission: Arc<PredictiveAdmission>,
    rpc: CowGrpcMethod,
}

impl Layer for PredAdmissionLayer {
    type Server = PredAdmissionServer;
    type Child = PredAdmissionChild;

    fn new(method: &CowGrpcMethod, server: &PredAdmissionServer, _ctx: &mut Context) -> Self {
        let resolved_method_id =
            MethodRegistry::global().get_or_register_method(method.service(), method.method());
        Self {
            est: EstRequestState::new(resolved_method_id, server.est.clone()),
            pred_admission: server.pred_admission.clone(),
            rpc: method.clone(),
        }
    }

    /// Reprioritize the current task based on remaining time to deadline.
    #[inline]
    fn before_poll<Ret>(&self, ctx: &Context) -> Result<(), Result<Response<Ret>, Status>> {
        #[cfg(feature = "sched_pred")]
        {
            let remaining = ctx.deadline().saturating_sub(masa_core::time_now());
            tokio::task::reprioritize(PriorityHint::new(remaining));
        }
        #[cfg(not(feature = "sched_pred"))]
        let _ = ctx;

        self.est.start_compute_tracking();
        Ok(())
    }

    /// Compute child deadline and priority, run predictive admission control,
    /// and set the child request context.
    ///
    /// Returns `Err` if the request should be shed.
    #[inline]
    fn before_child_rpc<T>(
        &self,
        ctx: &Context,
        child_method_name: &CowGrpcMethod,
        child_ctx: &mut PredAdmissionChild,
        _request: &mut tonic_core::Request<T>,
        child_rpc: &mut ChildRpcContext,
    ) -> Result<(), Status> {
        let est_remaining = {
            let result =
                self.est
                    .prepare_before_child_rpc(ctx, child_method_name, &mut child_ctx.est);

            if self.pred_admission.admission_check(
                &self.est.server,
                self.est.resolved_method_id,
                ctx,
                result.key,
            ) {
                return Err(Status::new(
                    Code::DeadlineExceeded,
                    format!(
                        "/EarlyReturn?src={}::{}?last_rpc={}::{}",
                        self.rpc.service(),
                        self.rpc.method(),
                        child_method_name.service(),
                        child_method_name.method(),
                    ),
                ));
            }

            result.est_remaining
        };

        let (deadline, prio_hint) = Self::child_deadline_and_prio(ctx, est_remaining);
        child_rpc.deadline = deadline;
        child_rpc.prio_hint = prio_hint;

        Ok(())
    }

    /// Process a child RPC response: track latencies, propagate errors.
    #[inline]
    fn after_child_rpc<T>(
        &self,
        ctx: &Context,
        _child_method: &CowGrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: &PredAdmissionChild,
    ) -> Result<(), Status> {
        child_ctx.est.finalize(response);
        if let Some(downstream_util) = self.est.after_child_rpc(response, &child_ctx.est) {
            self.pred_admission
                .update_bottleneck(ctx.api(), downstream_util);
        }

        #[cfg(feature = "sched_pred")]
        if let Err(status) = response {
            return Err(status.clone());
        }

        Ok(())
    }

    /// Stop compute tracking after a poll.
    #[inline]
    fn after_poll<Ret>(
        &self,
        _ctx: &Context,
        _poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        self.est.stop_compute_tracking();
        Ok(())
    }

    /// Track latencies and inject response metadata at finalization.
    ///
    /// Sets `response_meta` on the shared `ctx`; the caller serializes once.
    #[inline]
    fn finalize<Ret>(&self, ctx: &mut Context, result: &mut Result<Response<Ret>, Status>) {
        if !is_early_return_response(result) {
            self.est.track_latencies();
        }
        self.est.inject_response_meta(ctx);
    }
}

impl PredAdmissionLayer {
    /// Compute child deadline and priority hint.
    ///
    /// When `sched_pred` is enabled, tightens the deadline by subtracting
    /// `est_remaining`.  Uses deadline alone as `prio_hint` (not
    /// `deadline - est_child`) to avoid priority inversions under load.
    ///
    /// When `sched_pred` is disabled, passes through the parent values.
    #[inline]
    fn child_deadline_and_prio(ctx: &Context, est_remaining: u64) -> (u64, PriorityHint) {
        #[cfg(feature = "sched_pred")]
        {
            let d = ctx.deadline().saturating_sub(est_remaining);
            (d, PriorityHint::new(d))
        }
        #[cfg(not(feature = "sched_pred"))]
        {
            let _ = est_remaining;
            (ctx.deadline(), ctx.prio_hint())
        }
    }
}

// ── Per-Child-RPC ───────────────────────────────────────────────────────

/// Per-child-RPC predictive layer state.
#[derive(Debug, Clone)]
pub(crate) struct PredAdmissionChild {
    pub(crate) est: EstChildState<DefaultLatencyEstimator>,
}

impl LayerChild for PredAdmissionChild {
    fn new() -> Self {
        Self {
            est: EstChildState::new(),
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Predictive admission control
// ══════════════════════════════════════════════════════════════════════════

// ── Bottleneck Tracker ──────────────────────────────────────────────────

const STALENESS_SECS: f64 = 2.0;
const STALENESS_DEFAULT: f32 = 0.5;

/// Tracks max_downstream_util per API with staleness decay.
#[derive(Debug)]
pub(crate) struct BottleneckTracker {
    inner: Mutex<HashMap<String, (f32, Instant)>>,
}

impl BottleneckTracker {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn update(&self, api: &str, max_downstream_util: f32) {
        let mut map = self.inner.lock().unwrap();
        map.insert(api.to_string(), (max_downstream_util, Instant::now()));
    }

    pub(crate) fn get(&self, api: &str) -> f32 {
        let map = self.inner.lock().unwrap();
        if let Some((util, last_update)) = map.get(api) {
            let age = last_update.elapsed().as_secs_f64();
            if age > STALENESS_SECS {
                // Decay toward default as data becomes stale
                let decay = (-(age - STALENESS_SECS) / STALENESS_SECS).exp() as f32;
                *util * decay + STALENESS_DEFAULT * (1.0 - decay)
            } else {
                *util
            }
        } else {
            STALENESS_DEFAULT
        }
    }
}

// ── Admission Controller ────────────────────────────────────────────────

const UTIL_TARGET: f64 = 0.92;
const ADJUST_RATE: f64 = 0.5;
const MAX_BURST_SECS: f64 = 0.1;
const INITIAL_BUDGET_RATE: f64 = 10_000_000.0; // us/s — start generous

struct BudgetState {
    budget_us: f64,
    budget_rate: f64,
    last_refill: Instant,
}

impl std::fmt::Debug for BudgetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BudgetState")
            .field("budget_us", &self.budget_us)
            .field("budget_rate", &self.budget_rate)
            .finish()
    }
}

/// Admission controller using compute-budget token bucket.
#[derive(Debug)]
pub(crate) struct AdmissionController {
    bottleneck: BottleneckTracker,
    state: Mutex<BudgetState>,
}

impl AdmissionController {
    pub(crate) fn new() -> Self {
        Self {
            bottleneck: BottleneckTracker::new(),
            state: Mutex::new(BudgetState {
                budget_us: INITIAL_BUDGET_RATE * MAX_BURST_SECS,
                budget_rate: INITIAL_BUDGET_RATE,
                last_refill: Instant::now(),
            }),
        }
    }

    pub(crate) fn update_bottleneck(&self, api: &str, max_downstream_util: f32) {
        self.bottleneck.update(api, max_downstream_util);
    }

    /// Returns true if the request should be admitted.
    ///
    /// Uses a token-bucket where tokens are microseconds of compute budget.
    /// The refill rate adjusts up/down based on bottleneck utilization,
    /// similar to TCP congestion control discovering available bandwidth.
    pub(crate) fn should_admit(
        &self,
        api: &str,
        _time_left: u64,
        est_compute: u64,
        _est_total_mean: u64,
    ) -> bool {
        let bottleneck_util = self.bottleneck.get(api) as f64;

        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        state.last_refill = now;

        // Refill tokens, capped at burst limit
        state.budget_us += state.budget_rate * elapsed;
        let max_budget = state.budget_rate * MAX_BURST_SECS;
        if state.budget_us > max_budget {
            state.budget_us = max_budget;
        }

        // Adjust rate based on bottleneck utilization
        if bottleneck_util > UTIL_TARGET {
            state.budget_rate *= 1.0 - ADJUST_RATE * elapsed;
        } else {
            state.budget_rate *= 1.0 + ADJUST_RATE * elapsed;
        }
        // Don't let rate go negative or explode
        state.budget_rate = state.budget_rate.clamp(1.0, INITIAL_BUDGET_RATE * 10.0);

        // Admit if we have enough budget
        let cost = est_compute as f64;
        if state.budget_us >= cost {
            state.budget_us -= cost;
            true
        } else {
            false
        }
    }
}

// ── PredictiveAdmission ────────────────────────────────────────────────────────

// ac_pred ENABLED

#[cfg(feature = "ac_pred")]
#[derive(Debug)]
pub(crate) struct PredictiveAdmission {
    controller: AdmissionController,
}

#[cfg(feature = "ac_pred")]
impl PredictiveAdmission {
    pub(crate) fn new() -> Self {
        Self {
            controller: AdmissionController::new(),
        }
    }

    /// Two-layer admission check reading estimation maps from `est_server`.
    ///
    /// - Layer 1 (every hop): compute-time feasibility — reject if estimated
    ///   compute time exceeds remaining deadline.
    /// - Layer 2 (ingress only, hop_count==0): efficiency-based admission via
    ///   token-bucket `AdmissionController`.
    #[inline]
    pub(crate) fn admission_check(
        &self,
        est_server: &EstServerState<DefaultLatencyEstimator>,
        resolved_method_id: u64,
        ctx: &Context,
        key: u64,
    ) -> bool {
        use masa_core::time_now;

        let time_left = ctx.e2e_deadline().saturating_sub(time_now());

        // Layer 1: floor-based deadline feasibility
        let est_remaining_floor = est_server
            .est_after_child_latency
            .get_mean_floor_estimate(key)
            .unwrap_or(0)
            .min(time_left);
        if time_now() > ctx.e2e_deadline().saturating_sub(est_remaining_floor) {
            return true;
        }

        // Layer 2: compute-time feasibility (every hop)
        let est_compute_rem = est_server
            .est_compute_latency
            .get_estimate(resolved_method_id)
            .unwrap_or(0);
        if est_compute_rem > time_left {
            return true; // infeasible -> shed
        }

        // Layer 3: efficiency-based admission (ingress only)
        if ctx.hop_count() == 0 {
            let est_total_mean = est_server
                .est_after_child_latency
                .get_mean_estimate(key)
                .unwrap_or(0);
            let est_child = est_server
                .est_child_latency
                .get_estimate(key)
                .unwrap_or(est_compute_rem);
            if !self
                .controller
                .should_admit(ctx.api(), time_left, est_child, est_total_mean)
            {
                return true;
            }
        }

        false
    }

    /// Feed downstream utilization data into the admission controller.
    #[inline]
    pub(crate) fn update_bottleneck(&self, api: &str, max_downstream_util: f32) {
        self.controller.update_bottleneck(api, max_downstream_util);
    }
}

// ac_pred DISABLED

#[cfg(not(feature = "ac_pred"))]
#[derive(Debug)]
pub(crate) struct PredictiveAdmission;

#[cfg(not(feature = "ac_pred"))]
impl PredictiveAdmission {
    #[inline]
    pub(crate) fn new() -> Self {
        Self
    }

    /// Floor-based deadline feasibility check using latency estimates.
    ///
    /// When `ac_pred` is disabled, this is the only admission check that runs.
    #[inline]
    pub(crate) fn admission_check(
        &self,
        est_server: &EstServerState<DefaultLatencyEstimator>,
        _resolved_method_id: u64,
        ctx: &Context,
        key: u64,
    ) -> bool {
        use masa_core::time_now;

        let time_left = ctx.e2e_deadline().saturating_sub(time_now());
        let est_remaining_floor = est_server
            .est_after_child_latency
            .get_mean_floor_estimate(key)
            .unwrap_or(0)
            .min(time_left);
        time_now() > ctx.e2e_deadline().saturating_sub(est_remaining_floor)
    }

    #[inline]
    pub(crate) fn update_bottleneck(&self, _api: &str, _max_downstream_util: f32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bottleneck_tracker_default() {
        let tracker = BottleneckTracker::new();
        let util = tracker.get("unknown_api");
        assert!((util - STALENESS_DEFAULT).abs() < 0.01);
    }

    #[test]
    fn test_bottleneck_tracker_update() {
        let tracker = BottleneckTracker::new();
        tracker.update("Search", 0.9);
        let util = tracker.get("Search");
        assert!((util - 0.9).abs() < 0.01);
    }

    #[test]
    fn test_admission_controller_admits_with_budget() {
        let ac = AdmissionController::new();
        // With initial budget, small compute cost should be admitted
        let admitted = ac.should_admit("Search", 100_000, 1000, 50_000);
        assert!(admitted);
    }

    #[test]
    fn test_admission_controller_rejects_when_budget_exhausted() {
        let ac = AdmissionController::new();
        // Exhaust the budget by admitting requests with large compute costs
        // Initial budget = INITIAL_BUDGET_RATE * MAX_BURST_SECS = 10M * 0.1 = 1M us
        // Each request costs 100_000 us, so ~10 requests should exhaust it
        let mut rejected = false;
        for _ in 0..20 {
            if !ac.should_admit("Search", 100_000, 100_000, 50_000) {
                rejected = true;
                break;
            }
        }
        assert!(
            rejected,
            "should eventually reject when budget is exhausted"
        );
    }
}
