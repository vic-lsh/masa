// Predictive admission control layer — early-return-rate driven AC.
//
// When `ac_pred` is enabled, this layer runs admission control at ingress
// (hop_count == 0). It rejects requests probabilistically based on the
// observed early-return rate, with a goodput-derived floor that relaxes
// under overload.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use masa_core::Context;
use tonic_core::{Code, CowGrpcMethod, Response, Status};

use super::super::{ChildRpcContext, Layer, LayerChild, LayerServer};
use crate::layer::est::estimator::DefaultLatencyEstimator;
use crate::layer::est::latency_map::{MethodKey, ParentToChildKey};
use crate::layer::est::state::{is_early_return_response, LatencyEstimators};
use crate::policy_params::PolicyParams;
use crate::registry::MethodId;

// ── Server ──────────────────────────────────────────────────────────────

/// Server-level predictive admission state (shared across requests).
#[derive(Debug)]
pub(crate) struct PredAdmissionServer {
    pred_admission: Arc<PredictiveAdmission>,
}

impl LayerServer for PredAdmissionServer {
    fn new() -> Self {
        Self {
            pred_admission: Arc::new(PredictiveAdmission::new()),
        }
    }
}

// ── Per-Request ─────────────────────────────────────────────────────────

/// Per-request predictive admission layer state.
#[derive(Debug)]
pub(crate) struct PredAdmissionLayer {
    pred_admission: Arc<PredictiveAdmission>,
    est: LatencyEstimators<DefaultLatencyEstimator>,
    root_method_id: Option<MethodId>,
    rpc: CowGrpcMethod,
    /// Set when this layer rejects a child RPC. Prevents the rejection
    /// from feeding back into `er_rate` via `finalize`.
    self_rejected: AtomicBool,
}

impl Layer for PredAdmissionLayer {
    type Server = PredAdmissionServer;
    type Child = PredAdmissionChild;

    fn new(method: &CowGrpcMethod, server: &PredAdmissionServer, ctx: &mut Context) -> Self {
        let root_method_id = ctx.root_method().map(|rm| {
            crate::MethodRegistry::global()
                .get_or_register(CowGrpcMethod::new(rm.service.clone(), rm.method.clone()))
        });
        Self {
            pred_admission: server.pred_admission.clone(),
            est: crate::layer::estimation::global_estimators(),
            root_method_id,
            rpc: method.clone(),
            self_rejected: AtomicBool::new(false),
        }
    }

    /// Two-layer admission check before child RPC.
    ///
    /// - Layer 1 (every hop): floor-based deadline feasibility.
    /// - Layer 2 (ingress only): probabilistic rejection based on
    ///   early-return rate, floored by goodput health.
    #[inline]
    fn before_child_rpc<T>(
        &self,
        ctx: &Context,
        child_method_name: &CowGrpcMethod,
        _child_ctx: &mut PredAdmissionChild,
        _request: &mut tonic_core::Request<T>,
        _child_rpc: &mut ChildRpcContext,
    ) -> Result<(), Status> {
        use masa_core::time_now;

        let child_id = crate::MethodRegistry::global().get_or_register(child_method_name.clone());
        let key = ParentToChildKey::parent_rpc_method(
            crate::MethodRegistry::global().get_or_register(self.rpc.clone()),
        )
        .child_rpc_method(child_id);

        let time_left = ctx.e2e_deadline().saturating_sub(time_now());

        // Layer 1: floor-based deadline feasibility
        let remaining = self.est.est_after_child_wallclock(key, time_left);
        let est_child = self.est.est_child_wallclock(key).unwrap_or(0);
        if time_now() + est_child + remaining.floor > ctx.e2e_deadline() {
            return Err(Status::new(
                Code::DeadlineExceeded,
                format!(
                    "/EarlyReturn?src={}::{}?last_rpc={}::{}&reason=BeforeChildFeasibility",
                    self.rpc.service(),
                    self.rpc.method(),
                    child_method_name.service(),
                    child_method_name.method(),
                ),
            ));
        }

        // Layer 2: early-return-rate admission (ingress only)
        if ctx.hop_count() == 0 && !self.pred_admission.should_admit() {
            self.self_rejected.store(true, Ordering::Relaxed);
            return Err(Status::new(
                Code::DeadlineExceeded,
                format!(
                    "/EarlyReturn?src={}::{}?last_rpc={}::{}&reason=PredAdmissionRej",
                    self.rpc.service(),
                    self.rpc.method(),
                    child_method_name.service(),
                    child_method_name.method(),
                ),
            ));
        }

        Ok(())
    }

    /// Track subtree compute from child metadata.
    ///
    /// Predictive admission feedback is recorded once per ingress request in
    /// `finalize`, so root-local early returns contribute to the signal.
    #[inline]
    fn after_child_rpc<T>(
        &self,
        ctx: &Context,
        _child_method: &CowGrpcMethod,
        response: &mut Result<Response<T>, Status>,
        _child_ctx: &PredAdmissionChild,
    ) -> Result<(), Status> {
        use crate::context_ext::MasaResponseExt;

        if ctx.hop_count() == 0 {
            if let Ok(resp) = response {
                if let Some(child_ctx_resp) = resp.get_masa_context() {
                    if let Some(meta) = child_ctx_resp.response_meta() {
                        if let Some(root_mid) = self.root_method_id {
                            self.est.track_subtree_compute(
                                MethodKey(root_mid),
                                meta.accumulated_compute_us,
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[inline]
    fn finalize<Ret>(&self, ctx: &mut Context, result: &mut Result<Response<Ret>, Status>) {
        if ctx.hop_count() != 0 {
            return;
        }

        // Skip outcome recording for our own rejections — counting them
        // would cause er_rate to feed back on itself (reject → er_rate
        // rises → reject more → death spiral).
        if self.self_rejected.load(Ordering::Relaxed) {
            return;
        }

        // Learn from the final ingress outcome so local early returns such as
        // `LocalDeadlineExceeded` are visible to predictive admission.
        let is_er = is_early_return_response(result)
            || ctx
                .response_meta()
                .map(|meta| meta.early_return_count > 0)
                .unwrap_or(false);

        self.pred_admission.record_outcome(is_er);
    }
}

// ── Per-Child-RPC ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct PredAdmissionChild;

impl LayerChild for PredAdmissionChild {
    fn new() -> Self {
        Self
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Early-return-rate admission controller
// ══════════════════════════════════════════════════════════════════════════

struct AdmissionState {
    /// EMA of early-return fraction (0.0–1.0).
    er_rate: f64,
    /// Fast EMA of goodput (completions/s).
    goodput_fast: f64,
    /// Slow EMA of goodput (completions/s).
    goodput_slow: f64,
    /// Timestamp of last update.
    last_update: Instant,
    /// Completions since last update.
    completions: u64,
    /// Counter for periodic logging.
    log_counter: u64,
}

impl std::fmt::Debug for AdmissionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionState")
            .field("er_rate", &self.er_rate)
            .field("goodput_fast", &self.goodput_fast)
            .field("goodput_slow", &self.goodput_slow)
            .finish()
    }
}

/// Early-return-rate driven admission controller.
///
/// Rejects probabilistically at `er_rate`. When goodput is healthy
/// (fast EMA >= slow EMA), rejection is capped at `max_reject_floor`.
/// When goodput is dropping (overload), the cap lifts.
#[derive(Debug)]
pub(crate) struct PredictiveAdmission {
    state: Mutex<AdmissionState>,
}

impl PredictiveAdmission {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(AdmissionState {
                er_rate: 0.0,
                goodput_fast: 0.0,
                goodput_slow: 0.0,
                last_update: Instant::now(),
                completions: 0,
                log_counter: 0,
            }),
        }
    }

    /// Record whether a child RPC was an early return or a success.
    pub(crate) fn record_outcome(&self, is_early_return: bool) {
        let p = &PolicyParams::global().pred;
        let mut state = self.state.lock().unwrap();

        // Update early-return rate EMA (per-event).
        let er_sample = if is_early_return { 1.0 } else { 0.0 };
        state.er_rate += p.er_alpha * (er_sample - state.er_rate);

        // Track completions for goodput.
        if !is_early_return {
            state.completions += 1;
        }

        // Update goodput EMAs based on elapsed time.
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_update).as_secs_f64();
        if elapsed > 0.01 {
            let instant_goodput = state.completions as f64 / elapsed;
            state.completions = 0;
            state.last_update = now;

            let alpha_fast = 1.0 - (-elapsed / p.tau_fast).exp();
            let alpha_slow = 1.0 - (-elapsed / p.tau_slow).exp();
            state.goodput_fast += alpha_fast * (instant_goodput - state.goodput_fast);
            state.goodput_slow += alpha_slow * (instant_goodput - state.goodput_slow);
        }

        state.log_counter += 1;
        if state.log_counter % 1000 == 0 {
            let overloaded = state.goodput_fast < state.goodput_slow;
            log::info!(
                "[ac_pred] outcomes={} er_rate={:.4} goodput_fast={:.1} goodput_slow={:.1} overloaded={}",
                state.log_counter,
                state.er_rate,
                state.goodput_fast,
                state.goodput_slow,
                overloaded,
            );
        }
    }

    /// Returns true if the request should be admitted.
    pub(crate) fn should_admit(&self) -> bool {
        let p = &PolicyParams::global().pred;
        let state = self.state.lock().unwrap();

        let overloaded = state.goodput_fast < state.goodput_slow;
        let reject_prob = if overloaded {
            // Goodput dropping — overloaded, allow full rejection.
            state.er_rate
        } else {
            // Goodput healthy — cap rejection at floor.
            state.er_rate.min(p.max_reject_floor)
        };

        drop(state);
        let coin = rand::random::<f64>();
        let admitted = coin > reject_prob;
        if !admitted {
            log::debug!(
                "[ac_pred] REJECTED reject_prob={:.4} overloaded={}",
                reject_prob,
                overloaded,
            );
        }
        admitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admits_when_no_early_returns() {
        let ac = PredictiveAdmission::new();
        // No early returns recorded — er_rate is 0, should always admit.
        for _ in 0..100 {
            assert!(ac.should_admit());
        }
    }

    #[test]
    fn test_er_rate_increases_with_early_returns() {
        let ac = PredictiveAdmission::new();
        for _ in 0..100 {
            ac.record_outcome(true);
        }
        let state = ac.state.lock().unwrap();
        assert!(
            state.er_rate > 0.9,
            "er_rate should be near 1.0 after all early returns"
        );
    }

    #[test]
    fn test_er_rate_recovers_after_successes() {
        let ac = PredictiveAdmission::new();
        // Drive er_rate up.
        for _ in 0..100 {
            ac.record_outcome(true);
        }
        // Drive it back down.
        for _ in 0..200 {
            ac.record_outcome(false);
        }
        let state = ac.state.lock().unwrap();
        assert!(
            state.er_rate < 0.1,
            "er_rate should recover after successes"
        );
    }

    #[test]
    fn test_goodput_floor_caps_rejection() {
        let ac = PredictiveAdmission::new();
        // Record enough early returns to push er_rate high.
        for _ in 0..100 {
            ac.record_outcome(true);
        }
        // But also record successes with time gaps to make goodput_fast >= goodput_slow
        // (healthy state). In healthy state, rejection is capped at max_reject_floor.
        std::thread::sleep(std::time::Duration::from_millis(50));
        for _ in 0..50 {
            ac.record_outcome(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        for _ in 0..50 {
            ac.record_outcome(false);
        }

        // With healthy goodput, should admit most requests despite high er_rate.
        let mut admitted = 0;
        for _ in 0..1000 {
            if ac.should_admit() {
                admitted += 1;
            }
        }
        // max_reject_floor = 0.10, so ~90% should be admitted.
        assert!(
            admitted > 800,
            "should admit most requests when goodput is healthy, got {admitted}/1000"
        );
    }
}
