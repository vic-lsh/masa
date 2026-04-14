// Predictive admission control layer — AIMD-based admission control.
//
// When `ac_pred` is enabled, this layer runs admission control at ingress
// (hop_count == 0). It rejects requests probabilistically based on an
// AIMD-controlled admission probability (`admit_p`) that decreases when the
// observed ER fraction exceeds a threshold and recovers additively when healthy.
// Exponential idle decay opens admission naturally when traffic drops.

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
    /// Set when this layer rejects a request. Prevents the rejection
    /// from feeding back into the admission controller via `finalize`.
    self_rejected: AtomicBool,
    /// Guards the ingress admission check so it runs only on the first poll.
    admission_checked: AtomicBool,
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
            admission_checked: AtomicBool::new(false),
        }
    }

    /// Admission check at true ingress — runs once, before any handler work.
    ///
    /// Fires on the first poll of the request future (hop_count == 0 only).
    /// The `admission_checked` flag ensures it runs exactly once per request
    /// regardless of how many times the future is polled.
    #[inline]
    fn before_poll<Ret>(
        &self,
        ctx: &Context,
    ) -> Result<(), Result<tonic_core::Response<Ret>, Status>> {
        if ctx.hop_count() != 0 || self.admission_checked.swap(true, Ordering::Relaxed) {
            return Ok(());
        }
        if !self.pred_admission.should_admit() {
            self.self_rejected.store(true, Ordering::Relaxed);
            return Err(Err(Status::new(
                Code::DeadlineExceeded,
                format!(
                    "/EarlyReturn?src={}::{}&reason=PredAdmissionRej",
                    self.rpc.service(),
                    self.rpc.method(),
                ),
            )));
        }
        Ok(())
    }

    /// Floor-based deadline feasibility check before each child RPC.
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

        // Skip outcome recording for self-rejections to avoid the controller
        // feeding back on its own rejections (reject → admit_p drops further → reject more).
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
// AIMD admission controller
// ══════════════════════════════════════════════════════════════════════════

struct AdmissionState {
    /// Start of the current 50 ms observation window.
    last_update: Instant,
    /// When the window was last closed. Used by `should_admit` for idle decay:
    /// as this timestamp ages, reject_prob decays to 0, reopening admission
    /// automatically when traffic drops.
    er_last_update: Instant,
    /// Early-return events accumulated in the current window.
    er_count: u64,
    /// Total events (success + ER) accumulated in the current window.
    window_total: u64,
    /// Counter for periodic logging.
    log_counter: u64,
    /// AIMD-controlled admission probability [0.0, 1.0]. Starts at 1.0 (fully open).
    admit_p: f64,
}

impl std::fmt::Debug for AdmissionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionState")
            .field("admit_p", &self.admit_p)
            .finish()
    }
}

/// AIMD admission controller.
///
/// `admit_p` is updated per 50 ms window: multiplicative decrease (`admit_p *= beta`)
/// when the window's ER fraction exceeds `aimd_er_threshold`; additive increase
/// (`admit_p += alpha`) when healthy. Rejection probability is
/// `(1 - admit_p) * exp(-idle_elapsed / tau_er)` — the exponential term provides
/// natural phase reset when no outcomes arrive (idle traffic → admission opens).
#[derive(Debug)]
pub(crate) struct PredictiveAdmission {
    state: Mutex<AdmissionState>,
}

impl PredictiveAdmission {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(AdmissionState {
                last_update: Instant::now(),
                er_last_update: Instant::now(),
                er_count: 0,
                window_total: 0,
                log_counter: 0,
                admit_p: 1.0,
            }),
        }
    }

    /// Record whether an admitted request ended in an early return or a success.
    pub(crate) fn record_outcome(&self, is_early_return: bool) {
        let p = &PolicyParams::global().pred;
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();

        // Accumulate into the current 50 ms observation window.
        state.window_total += 1;
        if is_early_return {
            state.er_count += 1;
        }

        // At each 50 ms window boundary, compute the ER fraction and run the
        // AIMD step. Using a windowed fraction sample rather than per-event EMA:
        // - alpha is fixed per window regardless of how many events arrived
        // - sample variance scales as 1/N (more stable at high load)
        // - should_admit sees a frozen reject_prob for the full window, decoupling
        //   the admission decision from mid-window ER noise
        let elapsed = now.duration_since(state.last_update).as_secs_f64();
        if elapsed > 0.05 {
            let er_sample = state.er_count as f64 / state.window_total as f64;
            if er_sample > p.aimd_er_threshold {
                // Overloaded window: multiplicative decrease.
                state.admit_p = (state.admit_p * p.aimd_beta).max(0.0);
            } else {
                // Healthy window: additive increase, capped at 1.0.
                state.admit_p = (state.admit_p + p.aimd_alpha).min(1.0);
            }
            state.er_last_update = now;

            // Reset window.
            state.er_count = 0;
            state.window_total = 0;
            state.last_update = now;
        }

        state.log_counter += 1;
        if state.log_counter % 1000 == 0 {
            log::info!(
                "[ac_pred] outcomes={} admit_p={:.4}",
                state.log_counter,
                state.admit_p,
            );
        }
    }

    /// Returns true if the request should be admitted.
    pub(crate) fn should_admit(&self) -> bool {
        let p = &PolicyParams::global().pred;
        let state = self.state.lock().unwrap();

        // idle_decay decays reject_prob to 0 as time passes since the last window
        // close. This reopens admission automatically when traffic is sparse.
        let er_elapsed = Instant::now()
            .duration_since(state.er_last_update)
            .as_secs_f64();
        let idle_decay = (-er_elapsed / p.tau_er).exp();
        let reject_prob = ((1.0 - state.admit_p) * idle_decay).min(1.0);
        let admit_p = state.admit_p;
        drop(state);

        let coin = rand::random::<f64>();
        let admitted = coin > reject_prob;
        if !admitted {
            log::debug!(
                "[ac_pred] REJECTED reject_prob={:.4} admit_p={:.4}",
                reject_prob,
                admit_p
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
        // admit_p=1.0, idle_decay=1.0 → reject_prob=0 → always admit.
        for _ in 0..100 {
            assert!(ac.should_admit());
        }
    }

    #[test]
    fn test_admits_fully_when_no_overload() {
        let ac = PredictiveAdmission::new();
        // No outcomes recorded → admit_p stays 1.0 → should admit >95%.
        let admitted = (0..1000).filter(|_| ac.should_admit()).count();
        assert!(
            admitted > 950,
            "zero ER should admit >95%, got {admitted}/1000"
        );
    }

    /// Helper: backdate the window clock so the next `record_outcome` call
    /// crosses the 50 ms boundary and fires the AIMD step.
    fn backdate_window(ac: &PredictiveAdmission) {
        let mut state = ac.state.lock().unwrap();
        state.last_update = Instant::now() - std::time::Duration::from_millis(60);
    }

    #[test]
    fn test_aimd_decreases_admit_p_on_overloaded_window() {
        let ac = PredictiveAdmission::new();
        // Drive 10 all-ER windows (100% ER fraction > any reasonable threshold).
        for _ in 0..10 {
            backdate_window(&ac);
            ac.record_outcome(true);
        }
        let state = ac.state.lock().unwrap();
        assert!(
            state.admit_p < 1.0,
            "admit_p should decrease after overloaded windows, got {}",
            state.admit_p
        );
    }

    #[test]
    fn test_aimd_increases_admit_p_on_healthy_window() {
        let ac = PredictiveAdmission::new();
        // Set admit_p low, then drive all-success windows.
        {
            let mut state = ac.state.lock().unwrap();
            state.admit_p = 0.5;
        }
        for _ in 0..5 {
            backdate_window(&ac);
            ac.record_outcome(false);
        }
        let state = ac.state.lock().unwrap();
        assert!(
            state.admit_p > 0.5,
            "admit_p should increase after healthy windows, got {}",
            state.admit_p
        );
    }

    #[test]
    fn test_admit_p_starts_fully_open() {
        let ac = PredictiveAdmission::new();
        let state = ac.state.lock().unwrap();
        assert_eq!(state.admit_p, 1.0, "admit_p should start at 1.0");
    }

    /// Verify idle decay reopens admission when no outcomes arrive.
    ///
    /// Scenario: admit_p is driven low by overload, then traffic goes idle.
    /// reject_prob = (1 - admit_p) * exp(-idle / tau_er) should decay toward 0
    /// even though admit_p itself hasn't changed.
    #[test]
    fn test_idle_decay_reopens_admission() {
        let ac = PredictiveAdmission::new();

        // Set admit_p very low to force near-total rejection.
        {
            let mut state = ac.state.lock().unwrap();
            state.admit_p = 0.01;
            state.er_last_update = Instant::now();
        }

        let admitted_before = (0..1000).filter(|_| ac.should_admit()).count();
        assert!(
            admitted_before < 100,
            "low admit_p should cause frequent rejection, got {admitted_before}/1000"
        );

        // Simulate 3 × tau_er (= 6 s at default tau_er=2 s) of idle.
        // idle_decay = exp(-3) ≈ 0.05 → reject_prob ≈ 0.99 × 0.05 ≈ 0.05.
        {
            let mut state = ac.state.lock().unwrap();
            state.er_last_update = Instant::now() - std::time::Duration::from_secs(6);
        }

        let admitted_after = (0..1000).filter(|_| ac.should_admit()).count();
        assert!(
            admitted_after > 900,
            "idle decay should open admission after 3×tau_er, got {admitted_after}/1000"
        );
    }

}
