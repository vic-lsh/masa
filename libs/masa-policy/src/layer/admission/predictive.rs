// Predictive admission control layer — latency estimation, deadline
// tightening, and predictive admission control.
//
// When `estimator` is enabled, this layer tracks latency distributions,
// tightens child deadlines (when `sched_pred` is also enabled), and runs
// predictive admission control.

use std::sync::Arc;
#[cfg(feature = "ac_pred")]
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::task::Poll;
#[cfg(feature = "ac_pred")]
use std::time::Instant;

use masa_core::{Context, PriorityHint};
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::super::{ChildRpcContext, Layer, LayerChild, LayerServer};
use crate::layer::est::estimator::DefaultLatencyEstimator;
use crate::layer::est::state::{
    is_early_return_response, EstChildState, EstRequestState, EstServerState,
};
use crate::policy_params::PolicyParams;
use crate::MethodRegistry;

/// Result of the two-layer admission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdmissionResult {
    /// Request admitted.
    Admit,
    /// Shed by Layer 1 (deadline feasibility).
    ShedLayer1,
    /// Shed by Layer 2 (token bucket capacity).
    ShedLayer2,
}

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

            let admission = self.pred_admission.admission_check(
                &self.est.server,
                self.est.resolved_method_id,
                ctx,
                result.key,
            );
            if admission != AdmissionResult::Admit {
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

    /// Process a child RPC response: track latencies, accumulate goodput.
    #[inline]
    fn after_child_rpc<T>(
        &self,
        ctx: &Context,
        _child_method: &CowGrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: &PredAdmissionChild,
    ) -> Result<(), Status> {
        child_ctx.est.finalize(response);
        let _ = self.est.after_child_rpc(response, &child_ctx.est);

        // Accumulate completed cost for goodput tracking (ingress only).
        #[cfg(feature = "ac_pred")]
        if ctx.hop_count() == 0 {
            if response.is_ok() {
                if let Some(id) = &child_ctx.est.parent_to_child_id {
                    let cost = self
                        .est
                        .server
                        .est_child_latency
                        .get_estimate(id.to_key())
                        .unwrap_or(0);
                    self.pred_admission.record_completion(cost);
                }
            } else {
                self.pred_admission.record_early_return();
            }
        }

        #[cfg(not(feature = "ac_pred"))]
        let _ = ctx;

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
// Goodput-tracking admission control
// ══════════════════════════════════════════════════════════════════════════

// ── Admission Controller ────────────────────────────────────────────────

#[cfg(feature = "ac_pred")]
struct BudgetState {
    /// EMA of cost-weighted goodput (µs/s).
    goodput_rate: f64,
    /// Token bucket balance (µs).
    budget_us: f64,
    /// Timestamp of last admission check.
    last_update: Instant,
    /// EMA of early return rate (0.0 = no ERs, 1.0 = all ER).
    er_ema: f64,
}

#[cfg(feature = "ac_pred")]
impl std::fmt::Debug for BudgetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BudgetState")
            .field("goodput_rate", &self.goodput_rate)
            .field("budget_us", &self.budget_us)
            .field("er_ema", &self.er_ema)
            .finish()
    }
}

/// Goodput-tracking admission controller.
///
/// Sets the token-bucket refill rate to slightly above observed goodput
/// (successful completion throughput in µs/s). This eliminates the
/// observer effect of the previous utilization-based controller: goodput
/// reflects actual system output, not the controller's own actions.
#[cfg(feature = "ac_pred")]
#[derive(Debug)]
pub(crate) struct AdmissionController {
    state: Mutex<BudgetState>,
    /// Lock-free accumulator for completed child RPC costs (µs).
    /// Drained by `should_admit` to compute the instantaneous goodput rate.
    completed_cost_us: AtomicU64,
    /// Lock-free accumulator: count of early-returned completions.
    er_count: AtomicU64,
    /// Lock-free accumulator: count of all completions (success + ER).
    total_count: AtomicU64,
}

#[cfg(feature = "ac_pred")]
impl AdmissionController {
    pub(crate) fn new() -> Self {
        let p = &PolicyParams::global().pred;
        Self {
            state: Mutex::new(BudgetState {
                goodput_rate: p.initial_budget_rate,
                budget_us: p.initial_budget_rate * p.max_burst_secs,
                last_update: Instant::now(),
                er_ema: 0.0,
            }),
            completed_cost_us: AtomicU64::new(0),
            er_count: AtomicU64::new(0),
            total_count: AtomicU64::new(0),
        }
    }

    /// Record a successful child RPC completion (lock-free).
    pub(crate) fn record_completion(&self, est_child_cost: u64) {
        self.completed_cost_us
            .fetch_add(est_child_cost, Ordering::Relaxed);
        self.total_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an early-returned child RPC (lock-free).
    pub(crate) fn record_early_return(&self) {
        self.er_count.fetch_add(1, Ordering::Relaxed);
        self.total_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns true if the request should be admitted.
    ///
    /// Uses ER (early return) rate as the explore/exploit signal:
    /// - Explore mode (er_ema <= threshold): admit freely, skip budget check.
    /// - Exploit mode (er_ema > threshold): goodput-tracking token bucket.
    pub(crate) fn should_admit(&self, est_child_cost: u64) -> bool {
        let p = &PolicyParams::global().pred;
        let cost = est_child_cost as f64;

        // Drain accumulators (lock-free swap).
        let drained = self.completed_cost_us.swap(0, Ordering::Relaxed) as f64;
        let er_drained = self.er_count.swap(0, Ordering::Relaxed) as f64;
        let total_drained = self.total_count.swap(0, Ordering::Relaxed) as f64;

        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_update).as_secs_f64();
        state.last_update = now;

        // Update goodput EMA from accumulated completions.
        if elapsed > 0.0 {
            let instant_rate = drained / elapsed;
            let alpha = 1.0 - (-elapsed / p.tau).exp();
            state.goodput_rate += alpha * (instant_rate - state.goodput_rate);
        }

        // Update ER EMA from accumulated ER events.
        if total_drained > 0.0 {
            let er_rate = er_drained / total_drained;
            state.er_ema += p.rejection_alpha * (er_rate - state.er_ema);
        }

        // Explore mode: ER rate below threshold — admit freely, skip budget.
        if state.er_ema <= p.rejection_threshold {
            return true;
        }

        // Exploit mode: goodput-tracking budget.
        let budget_rate = state.goodput_rate * (1.0 + p.probe_min);

        state.budget_us += budget_rate * elapsed;
        let max_budget = (budget_rate * p.max_burst_secs).max(cost * 2.0);
        if state.budget_us > max_budget {
            state.budget_us = max_budget;
        }

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
    /// - Layer 1 (every hop): floor-based deadline feasibility — reject if
    ///   estimated remaining wall-clock time exceeds deadline.
    /// - Layer 2 (ingress only, hop_count==0): compute-capacity admission via
    ///   token-bucket, using the child's wall-clock latency as cost.
    #[inline]
    pub(crate) fn admission_check(
        &self,
        est_server: &EstServerState<DefaultLatencyEstimator>,
        _resolved_method_id: u64,
        ctx: &Context,
        key: u64,
    ) -> AdmissionResult {
        use masa_core::time_now;

        let time_left = ctx.e2e_deadline().saturating_sub(time_now());

        // Layer 1: floor-based deadline feasibility
        let est_remaining_floor = est_server
            .est_after_child_latency
            .get_mean_floor_estimate(key)
            .unwrap_or(0)
            .min(time_left);
        if time_now() > ctx.e2e_deadline().saturating_sub(est_remaining_floor) {
            return AdmissionResult::ShedLayer1;
        }

        // Layer 2: compute-capacity admission (ingress only)
        // Uses the child's wall-clock latency (from ResponseMeta, keyed by
        // parent→child pair) as cost.
        if ctx.hop_count() == 0 {
            let est_child_cost = est_server.est_child_latency.get_estimate(key).unwrap_or(0);
            if !self.controller.should_admit(est_child_cost) {
                return AdmissionResult::ShedLayer2;
            }
        }

        AdmissionResult::Admit
    }

    /// Record a successful child RPC completion for goodput tracking.
    #[inline]
    pub(crate) fn record_completion(&self, est_child_cost: u64) {
        self.controller.record_completion(est_child_cost);
    }

    /// Record an early-returned child RPC for ER rate tracking.
    #[inline]
    pub(crate) fn record_early_return(&self) {
        self.controller.record_early_return();
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
    /// Returns `ShedLayer1` or `Admit` (never `ShedLayer2`).
    #[inline]
    pub(crate) fn admission_check(
        &self,
        est_server: &EstServerState<DefaultLatencyEstimator>,
        _resolved_method_id: u64,
        ctx: &Context,
        key: u64,
    ) -> AdmissionResult {
        use masa_core::time_now;

        let time_left = ctx.e2e_deadline().saturating_sub(time_now());
        let est_remaining_floor = est_server
            .est_after_child_latency
            .get_mean_floor_estimate(key)
            .unwrap_or(0)
            .min(time_left);
        if time_now() > ctx.e2e_deadline().saturating_sub(est_remaining_floor) {
            AdmissionResult::ShedLayer1
        } else {
            AdmissionResult::Admit
        }
    }

    #[inline]
    pub(crate) fn record_completion(&self, _est_child_cost: u64) {}

    #[inline]
    pub(crate) fn record_early_return(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Push the admission controller into exploit mode by recording many ERs
    /// and draining the accumulators via `should_admit` calls.
    fn push_into_exploit_mode(ac: &AdmissionController) {
        // Record enough early returns to push er_ema above the threshold.
        // Each drain+update: er_ema += alpha * (1.0 - er_ema).
        // With rejection_alpha=0.01 and threshold=0.10, we need many rounds.
        for _ in 0..200 {
            ac.record_early_return();
            ac.should_admit(0);
        }
        // Verify we're in exploit mode by checking that the controller
        // can now reject (er_ema > threshold).
    }

    #[test]
    fn test_admission_controller_admits_in_explore_mode() {
        let ac = AdmissionController::new();
        // In explore mode (er_ema = 0.0 <= threshold), all requests admitted.
        for _ in 0..50 {
            assert!(ac.should_admit(100_000), "explore mode should always admit");
        }
    }

    #[test]
    fn test_admission_controller_rejects_when_budget_exhausted() {
        let ac = AdmissionController::new();
        // First, push into exploit mode so budget checking is active.
        push_into_exploit_mode(&ac);

        // Now exhaust the budget by admitting requests with large compute costs.
        // In exploit mode, budget_rate = goodput_rate * 1.05. With no real
        // completions (only ER records), goodput_rate has decayed toward 0.
        // Dynamic burst floor: max(budget_rate * 0.005, cost * 2) = 200K us.
        // Each request costs 100K, so ~2 requests exhaust the budget.
        let mut rejected = false;
        for _ in 0..20 {
            if !ac.should_admit(100_000) {
                rejected = true;
                break;
            }
        }
        assert!(
            rejected,
            "should eventually reject when budget is exhausted"
        );
    }

    #[test]
    fn test_goodput_tracking_refills_budget() {
        let ac = AdmissionController::new();
        // Push into exploit mode so budget checking is active.
        push_into_exploit_mode(&ac);

        // Exhaust budget
        while ac.should_admit(100_000) {}

        // Simulate completions (these also count as total_count successes)
        ac.record_completion(100_000);
        ac.record_completion(100_000);

        // Sleep to let elapsed time accumulate for refill
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Should be able to admit again after completions refill the budget
        assert!(
            ac.should_admit(1000),
            "should admit after completions refill the budget"
        );
    }
}
