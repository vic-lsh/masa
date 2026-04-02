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

    fn new(method: &CowGrpcMethod, server: &PredAdmissionServer, ctx: &mut Context) -> Self {
        let resolved_method_id =
            MethodRegistry::global().get_or_register_method(method.service(), method.method());
        // Set root_method at ingress (hop_count == 0)
        if ctx.hop_count() == 0 {
            ctx.root_method = resolved_method_id;
        }
        let root_method_id = ctx.root_method();
        Self {
            est: EstRequestState::new(resolved_method_id, root_method_id, server.est.clone()),
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
        let info = self.est.after_child_rpc(response, &child_ctx.est);

        // Accumulate completed cost for goodput tracking (ingress only, success only).
        // Uses accumulated compute cost from the child's entire subtree instead of
        // wall-clock child latency to avoid conflating queueing with compute.
        #[cfg(feature = "ac_pred")]
        if ctx.hop_count() == 0 && response.is_ok() {
            if let Some(acc_cost) = info.accumulated_compute_us {
                self.pred_admission.record_completion(acc_cost);
                let root = ctx.root_method();
                if root != 0 {
                    self.est.server.est_accumulated_cost.track(root, acc_cost);
                }
            }
        }

        #[cfg(not(feature = "ac_pred"))]
        let _ = (ctx, info);

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
    /// EMA of per-decision rejection rate (0.0 = no rejections, 1.0 = all rejected).
    rejection_ema: f64,
}

#[cfg(feature = "ac_pred")]
impl std::fmt::Debug for BudgetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BudgetState")
            .field("goodput_rate", &self.goodput_rate)
            .field("budget_us", &self.budget_us)
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
                rejection_ema: 0.0,
            }),
            completed_cost_us: AtomicU64::new(0),
        }
    }

    /// Record a successful child RPC completion (lock-free).
    pub(crate) fn record_completion(&self, est_child_cost: u64) {
        self.completed_cost_us
            .fetch_add(est_child_cost, Ordering::Relaxed);
    }

    /// Returns true if the request should be admitted.
    ///
    /// Uses a token-bucket where tokens are microseconds of compute budget.
    /// The refill rate tracks observed goodput plus a probe margin.
    pub(crate) fn should_admit(&self, est_child_cost: u64) -> bool {
        let p = &PolicyParams::global().pred;
        let cost = est_child_cost as f64;

        // Drain the completion accumulator (lock-free swap).
        let drained = self.completed_cost_us.swap(0, Ordering::Relaxed) as f64;

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

        // Exploit vs explore: when rejections are happening, track observed
        // goodput tightly; otherwise admit freely at the generous initial rate.
        let budget_rate = if state.rejection_ema > p.rejection_threshold {
            // Exploit mode: tight tracking of observed goodput
            state.goodput_rate * (1.0 + p.probe_min)
        } else {
            // Explore mode: admit freely using generous initial budget
            p.initial_budget_rate
        };

        // Refill tokens, capped at burst limit.
        // Dynamic floor: ensure the budget can always hold at least one
        // request (cost × 2), so no request is permanently inadmissible
        // regardless of how large est_child is (e.g., I/O-heavy Hotel calls).
        state.budget_us += budget_rate * elapsed;
        let max_budget = (budget_rate * p.max_burst_secs).max(cost * 2.0);
        if state.budget_us > max_budget {
            state.budget_us = max_budget;
        }

        // Admission decision + rejection EMA update.
        let rejected = if state.budget_us >= cost {
            state.budget_us -= cost;
            false
        } else {
            true
        };
        state.rejection_ema +=
            p.rejection_alpha * ((if rejected { 1.0 } else { 0.0 }) - state.rejection_ema);
        !rejected
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
        // Includes estimated child call duration so requests that will spend
        // most of their remaining budget on the child RPC are caught early.
        let est_remaining_floor = est_server
            .est_after_child_latency
            .get_mean_floor_estimate(key)
            .unwrap_or(0)
            .min(time_left);
        let est_child = est_server.est_child_latency.get_estimate(key).unwrap_or(0);
        if time_now() + est_child + est_remaining_floor > ctx.e2e_deadline() {
            return AdmissionResult::ShedLayer1;
        }

        // Layer 2: compute-capacity admission (ingress only)
        // Uses accumulated compute cost keyed by root API type. Falls back to
        // wall-clock child latency when root_method is not yet set (cold start).
        if ctx.hop_count() == 0 {
            let root = ctx.root_method();
            let est_cost = if root != 0 {
                est_server
                    .est_accumulated_cost
                    .get_estimate(root)
                    .unwrap_or(0)
            } else {
                est_server.est_child_latency.get_estimate(key).unwrap_or(0)
            };
            if !self.controller.should_admit(est_cost) {
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
        let est_child = est_server.est_child_latency.get_estimate(key).unwrap_or(0);
        if time_now() + est_child + est_remaining_floor > ctx.e2e_deadline() {
            AdmissionResult::ShedLayer1
        } else {
            AdmissionResult::Admit
        }
    }

    #[inline]
    pub(crate) fn record_completion(&self, _est_child_cost: u64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admission_controller_admits_with_budget() {
        let ac = AdmissionController::new();
        // With initial budget (goodput_rate = 5M, budget = 25K µs),
        // a small compute cost should be admitted.
        assert!(ac.should_admit(1000));
    }

    #[test]
    fn test_admission_controller_rejects_when_budget_exhausted() {
        let ac = AdmissionController::new();
        // Exhaust the budget by admitting requests with large compute costs.
        // Initial budget = 5M * 0.005 = 25K µs. With no completions,
        // goodput_rate decays toward 0 and budget won't refill meaningfully.
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
        // Exhaust budget
        while ac.should_admit(100_000) {}

        // Simulate completions
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
