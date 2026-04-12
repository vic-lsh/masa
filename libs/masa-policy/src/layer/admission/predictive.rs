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
    /// Set when this layer rejects a request. Prevents the rejection
    /// from feeding back into `er_rate` via `finalize`.
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
    /// Start of the current observation window (shared by goodput and er_rate).
    last_update: Instant,
    /// When er_rate was last updated from observed data.
    /// Used by `should_admit` to decay virt_er_rate when no outcomes arrive.
    er_last_update: Instant,
    /// Successful completions accumulated in the current window.
    completions: u64,
    /// Early-return events accumulated in the current window.
    er_count: u64,
    /// Total events (success + ER) accumulated in the current window.
    window_total: u64,
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
/// Rejects probabilistically at `virt_er_rate = er_rate * exp(-elapsed/tau_er)`.
/// The ER rate tracks the fraction of admitted requests that end in an early
/// return. Exponential decay provides natural phase reset when idle.
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
                er_last_update: Instant::now(),
                completions: 0,
                er_count: 0,
                window_total: 0,
                log_counter: 0,
            }),
        }
    }

    /// Record whether a child RPC was an early return or a success.
    pub(crate) fn record_outcome(&self, is_early_return: bool) {
        let p = &PolicyParams::global().pred;
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();

        // Accumulate into the current observation window.
        state.window_total += 1;
        if is_early_return {
            state.er_count += 1;
        } else {
            state.completions += 1;
        }

        // Both er_rate and goodput EMAs update on the same 10 ms window.
        //
        // Previously er_rate used a per-event Bernoulli update with a
        // time-corrected alpha. That is mathematically correct at steady load,
        // but during over-rejection (few admitted requests, sparse events)
        // the per-event alpha jumps to ~1-2% and a handful of consecutive
        // successes can collapse er_rate prematurely, triggering an admission
        // burst and a ping-pong cycle.
        //
        // Using a windowed fraction sample (er_count / window_total) instead:
        // - alpha is fixed per window (~0.005 at 10 ms / tau_er 2 s)
        //   regardless of how many events arrived
        // - sample variance scales as 1/N (lower at high load)
        // - behaviour is symmetric with the goodput window already in place
        let elapsed = now.duration_since(state.last_update).as_secs_f64();
        if elapsed > 0.01 {
            // Update goodput EMAs.
            let instant_goodput = state.completions as f64 / elapsed;
            let alpha_fast = 1.0 - (-elapsed / p.tau_fast).exp();
            let alpha_slow = 1.0 - (-elapsed / p.tau_slow).exp();
            state.goodput_fast += alpha_fast * (instant_goodput - state.goodput_fast);
            state.goodput_slow += alpha_slow * (instant_goodput - state.goodput_slow);

            // Update er_rate EMA from the window fraction.
            let er_sample = state.er_count as f64 / state.window_total as f64;
            let alpha_er = 1.0 - (-elapsed / p.tau_er).exp();
            state.er_rate += alpha_er * (er_sample - state.er_rate);
            state.er_last_update = now;

            // Reset window.
            state.completions = 0;
            state.er_count = 0;
            state.window_total = 0;
            state.last_update = now;
        }

        state.log_counter += 1;
        if state.log_counter % 1000 == 0 {
            let overloaded = state.goodput_fast < state.goodput_slow; // approx; exact check in should_admit
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

        // Apply time-based decay to er_rate to get the virtual rejection
        // probability. This provides adaptive phase reset: when arrivals drop
        // (or self-rejections dominate so record_outcome is never called),
        // stale er_rate fades within ~tau_er seconds without any explicit reset.
        let er_elapsed = Instant::now()
            .duration_since(state.er_last_update)
            .as_secs_f64();
        let virt_er_rate = state.er_rate * (-er_elapsed / p.tau_er).exp();

        // Derivative term: fires when goodput is falling (goodput_fast < goodput_slow),
        // which happens within ~tau_fast seconds of a load spike before ER rate builds
        // up over ~tau_er seconds. Vanishes at steady state so equilibrium and loop
        // gain are unchanged.
        let goodput_divergence = if state.goodput_slow > 0.0 {
            ((state.goodput_slow - state.goodput_fast) / state.goodput_slow).max(0.0)
        } else {
            0.0
        };
        let reject_prob = (virt_er_rate * p.reject_scale
            + goodput_divergence * p.goodput_divergence_weight)
            .min(1.0);

        drop(state);
        let coin = rand::random::<f64>();
        let admitted = coin > reject_prob;
        if !admitted {
            log::debug!(
                "[ac_pred] REJECTED reject_prob={:.4} virt_er={:.4} div={:.4}",
                reject_prob,
                virt_er_rate,
                goodput_divergence
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

    /// Helper: backdate `last_update` (the window clock) so the next
    /// `record_outcome` call fires the 10 ms window and applies a meaningful
    /// alpha to er_rate (50 ms elapsed → alpha_er ≈ 0.025 at tau_er = 2 s).
    fn backdate_window(ac: &PredictiveAdmission) {
        let mut state = ac.state.lock().unwrap();
        state.last_update = Instant::now() - std::time::Duration::from_millis(50);
    }

    #[test]
    fn test_er_rate_increases_with_early_returns() {
        let ac = PredictiveAdmission::new();
        // Backdate window before each call so every event fires its own window
        // with a 50 ms elapsed → alpha_er ≈ 0.025. After 10 all-ER windows:
        // er_rate ≈ 1 - 0.975^10 ≈ 0.22; after 50 windows ≈ 0.72. Use enough
        // iterations (100) to drive er_rate reliably above 0.9.
        for _ in 0..100 {
            backdate_window(&ac);
            ac.record_outcome(true);
        }
        let state = ac.state.lock().unwrap();
        assert!(
            state.er_rate > 0.9,
            "er_rate should be near 1.0 after many early returns, got {}",
            state.er_rate
        );
    }

    #[test]
    fn test_er_rate_recovers_after_successes() {
        let ac = PredictiveAdmission::new();
        // Drive er_rate up.
        for _ in 0..100 {
            backdate_window(&ac);
            ac.record_outcome(true);
        }
        // Drive it back down with success windows. Each window (50 ms, sample=0):
        // er_rate *= (1 - 0.025). After ~100 success windows er_rate < 0.1.
        for _ in 0..100 {
            backdate_window(&ac);
            ac.record_outcome(false);
        }
        let state = ac.state.lock().unwrap();
        assert!(
            state.er_rate < 0.1,
            "er_rate should recover after successes, got {}",
            state.er_rate
        );
    }

    #[test]
    fn test_admits_when_er_rate_low() {
        let ac = PredictiveAdmission::new();
        // er_rate = 0 (no early returns) → virt_er_rate ≈ 0 → should admit almost all.
        let admitted = (0..1000).filter(|_| ac.should_admit()).count();
        assert!(
            admitted > 950,
            "zero er_rate should admit >95%, got {admitted}/1000"
        );
    }

    #[test]
    fn test_rejects_proportional_to_er_rate() {
        let ac = PredictiveAdmission::new();
        // Drive er_rate high via windowed backdating.
        for _ in 0..100 {
            backdate_window(&ac);
            ac.record_outcome(true);
        }
        // Even with goodput_fast >= goodput_slow (healthy), should reject heavily
        // because rejection is driven by virt_er_rate alone.
        {
            let mut state = ac.state.lock().unwrap();
            state.goodput_fast = 200.0;
            state.goodput_slow = 150.0; // healthy: fast > slow
        }
        let admitted = (0..1000).filter(|_| ac.should_admit()).count();
        assert!(
            admitted < 100,
            "high er_rate should reject ~99% regardless of healthy goodput, got {admitted}/1000"
        );
    }

    /// Verifies that er_rate decays naturally when no completions arrive,
    /// providing automatic phase reset without explicit state manipulation.
    ///
    /// Scenario: system built up high er_rate under load, then load drops.
    /// Even without any new record_outcome calls, should_admit should become
    /// progressively more permissive as virt_er_rate = er_rate * exp(-t/tau_er)
    /// decays toward zero.
    #[test]
    fn test_er_rate_decays_when_idle() {
        let ac = PredictiveAdmission::new();

        // Build high er_rate via windowed backdating.
        for _ in 0..100 {
            backdate_window(&ac);
            ac.record_outcome(true);
        }

        // Sanity: er_rate ≈ 0.99 → virt_er_rate ≈ 0.99 → almost all rejected.
        let admitted_before = (0..1000).filter(|_| ac.should_admit()).count();
        assert!(
            admitted_before < 200,
            "high er_rate with overloaded state should cause frequent rejection, got {admitted_before}/1000"
        );

        // Simulate idle period: backdate er_last_update by 3 × tau_er (= 6 s).
        // virt_er_rate = 0.99 × exp(-3) ≈ 0.05 — effectively near zero.
        {
            let mut state = ac.state.lock().unwrap();
            state.er_last_update = Instant::now() - std::time::Duration::from_secs(6);
        }

        // After idle period er_rate decays → almost all admitted.
        let admitted_after = (0..1000).filter(|_| ac.should_admit()).count();
        assert!(
            admitted_after > 900,
            "er_rate should decay to near zero after 3×tau_er idle, got {admitted_after}/1000"
        );
    }

    /// Verify reject_scale defaults to 1.0 and is included in the compiled params.
    #[test]
    fn test_reject_scale_default() {
        let p = crate::policy_params::PolicyParams::default();
        assert_eq!(p.pred.reject_scale, 1.0);
    }

    #[test]
    fn test_goodput_divergence_weight_default() {
        let p = crate::policy_params::PolicyParams::default();
        assert_eq!(p.pred.goodput_divergence_weight, 0.0);
    }
}
