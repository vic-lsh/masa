// Predictive admission control layer — latency estimation, deadline
// tightening, and predictive admission control.
//
// When `estimator` is enabled, this layer tracks latency distributions,
// tightens child deadlines (when `sched_pred` is also enabled), and runs
// predictive admission control.

use std::sync::Arc;
#[cfg(feature = "ac_pred")]
use std::sync::{
    atomic::{AtomicU32, Ordering},
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

        // Track concurrency: decrement in-flight and update latency (ingress only).
        #[cfg(feature = "ac_pred")]
        if ctx.hop_count() == 0 {
            if response.is_ok() {
                if let Some(start) = child_ctx.est.start_time {
                    let latency_us = Instant::now().duration_since(start).as_micros() as u64;
                    self.pred_admission.record_completion(latency_us);
                } else {
                    self.pred_admission.record_drop();
                }
            } else {
                self.pred_admission.record_drop();
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

// ── Concurrency Limiter ─────────────────────────────────────────────────

#[cfg(feature = "ac_pred")]
struct LimiterState {
    /// Current adaptive concurrency limit.
    limit: f64,
    /// Observed no-load latency floor (µs). Zero until first completion.
    min_latency_us: f64,
    /// EMA of recent latencies (µs). Zero until first completion.
    avg_latency_us: f64,
    /// Timestamp of last completion (for time-based EMA alpha).
    last_update: Instant,
    /// Timestamp of last AIMD limit update.
    last_limit_update: Instant,
}

#[cfg(feature = "ac_pred")]
impl std::fmt::Debug for LimiterState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LimiterState")
            .field("limit", &self.limit)
            .field("min_latency_us", &self.min_latency_us)
            .field("avg_latency_us", &self.avg_latency_us)
            .finish()
    }
}

/// AIMD concurrency limiter.
///
/// Controls the number of in-flight requests at ingress, adapting the
/// limit using a latency-based gradient: `gradient = min_latency / avg_latency`.
/// When the gradient is above the threshold (healthy), the limit increases
/// additively. When below (congested), it decreases multiplicatively.
#[cfg(feature = "ac_pred")]
#[derive(Debug)]
pub(crate) struct ConcurrencyLimiter {
    /// Current in-flight request count.
    inflight: AtomicU32,
    state: Mutex<LimiterState>,
}

#[cfg(feature = "ac_pred")]
impl ConcurrencyLimiter {
    pub(crate) fn new() -> Self {
        let p = &PolicyParams::global().pred;
        Self {
            inflight: AtomicU32::new(0),
            state: Mutex::new(LimiterState {
                limit: p.initial_limit,
                min_latency_us: 0.0,
                avg_latency_us: 0.0,
                last_update: Instant::now(),
                last_limit_update: Instant::now(),
            }),
        }
    }

    /// Check if a new request should be admitted.
    ///
    /// If admitted, increments the in-flight count. The caller MUST
    /// eventually call `record_completion` or `record_drop`.
    pub(crate) fn should_admit(&self) -> bool {
        let limit = {
            let state = self.state.lock().unwrap();
            state.limit
        };
        let current = self.inflight.fetch_add(1, Ordering::Relaxed);
        if (current as f64) < limit {
            true
        } else {
            self.inflight.fetch_sub(1, Ordering::Relaxed);
            false
        }
    }

    /// Record a successful completion: decrement in-flight, update latency
    /// estimates, and adapt the concurrency limit.
    pub(crate) fn record_completion(&self, latency_us: u64) {
        self.inflight.fetch_sub(1, Ordering::Relaxed);

        let p = &PolicyParams::global().pred;
        let latency = latency_us as f64;
        if latency <= 0.0 {
            return;
        }

        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_update).as_secs_f64();
        state.last_update = now;

        // Update min_latency: adopt new lows instantly, decay upward slowly.
        if state.min_latency_us <= 0.0 || latency < state.min_latency_us {
            state.min_latency_us = latency;
        } else {
            state.min_latency_us += p.min_latency_alpha * (latency - state.min_latency_us);
        }

        // Update avg_latency EMA.
        if state.avg_latency_us <= 0.0 {
            state.avg_latency_us = latency;
        } else if elapsed > 0.0 {
            let alpha = 1.0 - (-elapsed / p.avg_latency_tau).exp();
            state.avg_latency_us += alpha * (latency - state.avg_latency_us);
        }

        // Periodic AIMD limit update.
        let update_elapsed = now.duration_since(state.last_limit_update).as_secs_f64();
        if update_elapsed >= (p.update_interval_ms as f64 / 1000.0) {
            state.last_limit_update = now;
            if state.min_latency_us > 0.0 && state.avg_latency_us > 0.0 {
                let gradient = state.min_latency_us / state.avg_latency_us;
                if gradient >= p.gradient_threshold {
                    // Healthy: additive increase
                    state.limit = (state.limit + p.additive_increase).min(p.max_limit);
                } else {
                    // Congested: multiplicative decrease
                    state.limit = (state.limit * p.multiplicative_decrease).max(10.0);
                }
            }
        }
    }

    /// Record a failed/early-return completion: decrement in-flight without
    /// updating latency estimates (failed requests have atypical latency).
    pub(crate) fn record_drop(&self) {
        self.inflight.fetch_sub(1, Ordering::Relaxed);
    }
}

// ── PredictiveAdmission ────────────────────────────────────────────────────────

// ac_pred ENABLED

#[cfg(feature = "ac_pred")]
#[derive(Debug)]
pub(crate) struct PredictiveAdmission {
    controller: ConcurrencyLimiter,
}

#[cfg(feature = "ac_pred")]
impl PredictiveAdmission {
    pub(crate) fn new() -> Self {
        Self {
            controller: ConcurrencyLimiter::new(),
        }
    }

    /// Two-layer admission check reading estimation maps from `est_server`.
    ///
    /// - Layer 1 (every hop): floor-based deadline feasibility — reject if
    ///   estimated remaining wall-clock time exceeds deadline.
    /// - Layer 2 (ingress only, hop_count==0): concurrency-based admission —
    ///   reject if in-flight count exceeds the adaptive limit.
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

        // Layer 2: concurrency-based admission (ingress only)
        if ctx.hop_count() == 0 {
            if !self.controller.should_admit() {
                return AdmissionResult::ShedLayer2;
            }
        }

        AdmissionResult::Admit
    }

    /// Record a successful child RPC completion with observed latency.
    #[inline]
    pub(crate) fn record_completion(&self, latency_us: u64) {
        self.controller.record_completion(latency_us);
    }

    /// Record a failed/early-return completion (decrement in-flight only).
    #[inline]
    pub(crate) fn record_drop(&self) {
        self.controller.record_drop();
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
    pub(crate) fn record_completion(&self, _latency_us: u64) {}

    #[inline]
    pub(crate) fn record_drop(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_concurrency_limiter_admits_under_limit() {
        let cl = ConcurrencyLimiter::new();
        // With initial_limit = 100, first request should be admitted.
        assert!(cl.should_admit());
        assert_eq!(cl.inflight.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_concurrency_limiter_rejects_at_limit() {
        let cl = ConcurrencyLimiter::new();
        // Default initial_limit = 100. Admit 100 requests.
        for _ in 0..100 {
            assert!(cl.should_admit());
        }
        // 101st should be rejected.
        assert!(
            !cl.should_admit(),
            "should reject when at concurrency limit"
        );
        assert_eq!(cl.inflight.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn test_completion_decrements_and_readmits() {
        let cl = ConcurrencyLimiter::new();
        // Fill to limit.
        for _ in 0..100 {
            assert!(cl.should_admit());
        }
        assert!(!cl.should_admit());

        // Complete one request.
        cl.record_completion(25_000); // 25ms latency
        assert_eq!(cl.inflight.load(Ordering::Relaxed), 99);

        // Should admit again.
        assert!(cl.should_admit());
    }

    #[test]
    fn test_drop_decrements_without_latency_update() {
        let cl = ConcurrencyLimiter::new();
        assert!(cl.should_admit());
        assert_eq!(cl.inflight.load(Ordering::Relaxed), 1);

        cl.record_drop();
        assert_eq!(cl.inflight.load(Ordering::Relaxed), 0);

        // Latency state should still be at initial values.
        let state = cl.state.lock().unwrap();
        assert_eq!(state.min_latency_us, 0.0);
        assert_eq!(state.avg_latency_us, 0.0);
    }

    #[test]
    fn test_limit_shrinks_under_latency_inflation() {
        let cl = ConcurrencyLimiter::new();
        let initial_limit = { cl.state.lock().unwrap().limit };

        // First completion sets the floor.
        cl.record_completion(10_000); // 10ms

        // Sleep briefly so EMA alpha is non-trivial.
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Completions with much higher latency (queueing).
        for _ in 0..20 {
            cl.record_completion(50_000); // 50ms — 5x the floor
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // Wait for the AIMD update interval to elapse (default 250ms),
        // then trigger the periodic update with one more completion.
        std::thread::sleep(std::time::Duration::from_millis(260));
        cl.record_completion(50_000);

        let final_limit = cl.state.lock().unwrap().limit;
        assert!(
            final_limit < initial_limit,
            "limit should shrink when latency is inflated: {} < {}",
            final_limit,
            initial_limit,
        );
    }
}
