// Predictive admission control layer — AIMD-based admission control.
//
// When `ac_pred` is enabled, this layer runs admission control at ingress
// (hop_count == 0). It rejects requests probabilistically based on an
// AIMD-controlled admission probability (`admit_p`) that decreases when the
// observed ER fraction exceeds a threshold and recovers additively when healthy.
// Exponential idle decay opens admission naturally when traffic drops.

use std::collections::HashMap;
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

// ── Tunables specific to the BCF feasibility check ──────────────────────
//
// Generic decay primitives live in `crate::layer::est::state` so the
// estimation layer (deadline tightening, priority assignment) can apply the
// same staleness model.

use crate::layer::est::state::{decay_factor, fast_exp_neg};

/// Steepness of the probabilistic shed sigmoid: `shed_prob = 1 - exp(-LAMBDA * overshoot/time_left)`.
/// At `ratio = 1.0` (overshoot equal to remaining budget), sheds ~86%; at 0.1, ~18%;
/// at 5.0, ~99%. Never reaches exactly 1.0, so a trickle of admissions always
/// flows through to refresh the EMA.
const LAMBDA: f64 = 2.0;

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
        let root_id = self
            .root_method_id
            .unwrap_or_else(|| crate::MethodRegistry::global().get_or_register(self.rpc.clone()));
        if !self.pred_admission.should_admit(root_id) {
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

    /// Time-decay-aware probabilistic feasibility check before each child RPC.
    ///
    /// Two gates:
    ///   1. Hard floor backstop (deterministic): aborts only when even the lower
    ///      envelope of observed wallclock places completion past the deadline.
    ///   2. Probabilistic mean-based shed: above the deadline by `overshoot`,
    ///      shed with probability `1 - exp(-LAMBDA * overshoot / time_left)`.
    ///
    /// Both gates apply wallclock-time decay to the EMA estimates: stale samples
    /// (no fresh observation in `TAU_DECAY_US`) shrink toward zero, breaking the
    /// metastable lockout where AIMD rejects everything → no fresh observations →
    /// estimates stay frozen at spike values.
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
        let parent_id = crate::MethodRegistry::global().get_or_register(self.rpc.clone());
        let root_id = self.root_method_id.unwrap_or(parent_id);
        let key = ParentToChildKey::root_rpc_method(root_id)
            .parent_rpc_method(parent_id)
            .child_rpc_method(child_id);

        let now = time_now();
        let deadline = ctx.e2e_deadline();
        let time_left = deadline.saturating_sub(now);
        if time_left == 0 {
            return Err(Self::bcf_error(&self.rpc, child_method_name));
        }

        // One pack lookup for child wallclock (mean, floor, last_obs).
        let child = self.est.child_wallclock_pack(key).unwrap_or_default();
        // Existing fanout-aware lookup for the after-child wallclock.
        let remaining = self.est.est_after_child_wallclock(key, time_left);
        let remaining_last_obs = self.est.after_child_wallclock_last_obs(key);

        // Cold start: no observations on either side → admit unconditionally.
        if child.mean == 0 && remaining.mean == 0 {
            return Ok(());
        }

        // Compute decay once and apply it to BOTH gates. Without the decay, a
        // floor estimate that ratchets up under sustained queueing would lock
        // out admission once it crosses the deadline, even after load drops —
        // the same metastable trap the redesign exists to break.
        let decay = {
            let child_decay = decay_factor(now, child.last_observation_us);
            let rem_decay = decay_factor(now, remaining_last_obs);
            child_decay.min(rem_decay)
        };

        // Gate 1: hard backstop on decayed floors. Aborts only when even the
        // (decayed) lower envelope places completion past the deadline — a true
        // "no chance" signal under current freshness.
        let floor_finish_us =
            now as f64 + (child.floor as f64 + remaining.floor as f64) * decay;
        if floor_finish_us > deadline as f64 {
            return Err(Self::bcf_error(&self.rpc, child_method_name));
        }

        // Fast path: even the raw means fit (decay ≈ 1 here too — covered by the
        // same comparison without recomputation). No exp, no rand.
        let raw_finish = now + child.mean + remaining.mean;
        if raw_finish <= deadline {
            return Ok(());
        }

        // Apply decay to the mean-based projection for the probabilistic shed.
        let decayed_finish = if decay < 1.0 {
            now + ((child.mean as f64 + remaining.mean as f64) * decay) as u64
        } else {
            raw_finish
        };

        if decayed_finish <= deadline {
            return Ok(());
        }

        // Gate 2: probabilistic shed proportional to mean-based overshoot ratio.
        let overshoot = decayed_finish - deadline;
        let ratio = overshoot as f64 / time_left as f64;
        let shed_prob = 1.0 - fast_exp_neg(LAMBDA * ratio);
        if rand::random::<f64>() < shed_prob {
            return Err(Self::bcf_error(&self.rpc, child_method_name));
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

        let root_id = self
            .root_method_id
            .unwrap_or_else(|| crate::MethodRegistry::global().get_or_register(self.rpc.clone()));
        self.pred_admission.record_outcome(root_id, is_er);
    }
}

impl PredAdmissionLayer {
    /// Build the `BeforeChildFeasibility` early-return status. Factored out so
    /// both the floor-backstop and probabilistic-shed paths emit identical
    /// error envelopes.
    #[inline]
    fn bcf_error(parent_rpc: &CowGrpcMethod, child_rpc: &CowGrpcMethod) -> Status {
        Status::new(
            Code::DeadlineExceeded,
            format!(
                "/EarlyReturn?src={}::{}?last_rpc={}::{}&reason=BeforeChildFeasibility",
                parent_rpc.service(),
                parent_rpc.method(),
                child_rpc.service(),
                child_rpc.method(),
            ),
        )
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

impl AdmissionState {
    fn new(now: Instant) -> Self {
        Self {
            last_update: now,
            er_last_update: now,
            er_count: 0,
            window_total: 0,
            log_counter: 0,
            admit_p: 1.0,
        }
    }
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
/// Maintains both a global AIMD state and per-root-method AIMD states. The
/// global state preserves a shared overload cap across APIs, while the per-root
/// state lets APIs with different SLOs and ER behavior become more protective
/// independently.
#[derive(Debug)]
pub(crate) struct PredictiveAdmission {
    global_state: Mutex<AdmissionState>,
    states_by_root: Mutex<HashMap<MethodId, AdmissionState>>,
}

impl PredictiveAdmission {
    /// How much root-specific pressure can raise rejection probability above
    /// the global controller. A full max overreacts at moderate overload; a
    /// partial lift keeps root feedback useful without letting one API fully
    /// veto admission on its own.
    const ROOT_REJECT_EXTRA_FRACTION: f64 = 0.75;

    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self {
            global_state: Mutex::new(AdmissionState::new(now)),
            states_by_root: Mutex::new(HashMap::new()),
        }
    }

    /// Record whether an admitted request ended in an early return or a success.
    pub(crate) fn record_outcome(&self, root_id: MethodId, is_early_return: bool) {
        let p = &PolicyParams::global().pred;
        let now = Instant::now();

        {
            let mut state = self.global_state.lock().unwrap();
            Self::record_outcome_for_state(&mut state, p, now, is_early_return);
            if state.log_counter % 1000 == 0 {
                log::info!(
                    "[ac_pred] scope=global outcomes={} admit_p={:.4}",
                    state.log_counter,
                    state.admit_p,
                );
            }
        }

        let mut states = self.states_by_root.lock().unwrap();
        let state = states
            .entry(root_id)
            .or_insert_with(|| AdmissionState::new(now));
        Self::record_outcome_for_state(state, p, now, is_early_return);
        if state.log_counter % 1000 == 0 {
            log::info!(
                "[ac_pred] scope=root root={:?} outcomes={} admit_p={:.4}",
                root_id,
                state.log_counter,
                state.admit_p,
            );
        }
    }

    /// Returns true if the request should be admitted.
    pub(crate) fn should_admit(&self, root_id: MethodId) -> bool {
        let p = &PolicyParams::global().pred;
        let now = Instant::now();

        let (global_reject_prob, global_admit_p) = {
            let state = self.global_state.lock().unwrap();
            (Self::reject_prob_for_state(&state, p, now), state.admit_p)
        };

        let mut states = self.states_by_root.lock().unwrap();
        let state = states
            .entry(root_id)
            .or_insert_with(|| AdmissionState::new(now));
        let root_reject_prob = Self::reject_prob_for_state(state, p, now);
        let root_admit_p = state.admit_p;
        drop(states);

        let root_extra = (root_reject_prob - global_reject_prob).max(0.0);
        let reject_prob =
            (global_reject_prob + Self::ROOT_REJECT_EXTRA_FRACTION * root_extra).min(1.0);

        let coin = rand::random::<f64>();
        let admitted = coin > reject_prob;
        if !admitted {
            log::debug!(
                "[ac_pred] REJECTED reject_prob={:.4} global_admit_p={:.4} root={:?} root_admit_p={:.4}",
                reject_prob,
                global_admit_p,
                root_id,
                root_admit_p
            );
        }
        admitted
    }

    fn record_outcome_for_state(
        state: &mut AdmissionState,
        p: &crate::policy_params::PredParams,
        now: Instant,
        is_early_return: bool,
    ) {
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
    }

    fn reject_prob_for_state(
        state: &AdmissionState,
        p: &crate::policy_params::PredParams,
        now: Instant,
    ) -> f64 {
        // idle_decay decays reject_prob to 0 as time passes since the last window
        // close. This reopens admission automatically when traffic is sparse.
        let er_elapsed = now.duration_since(state.er_last_update).as_secs_f64();
        let idle_decay = (-er_elapsed / p.tau_er).exp();
        ((1.0 - state.admit_p) * idle_decay).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(service: &'static str) -> MethodId {
        crate::MethodRegistry::global().get_or_register(CowGrpcMethod::new(service, "Root"))
    }

    #[test]
    fn test_admits_when_no_early_returns() {
        let ac = PredictiveAdmission::new();
        let root = test_root("test_admits_when_no_early_returns");
        // admit_p=1.0, idle_decay=1.0 → reject_prob=0 → always admit.
        for _ in 0..100 {
            assert!(ac.should_admit(root));
        }
    }

    #[test]
    fn test_admits_fully_when_no_overload() {
        let ac = PredictiveAdmission::new();
        let root = test_root("test_admits_fully_when_no_overload");
        // No outcomes recorded → admit_p stays 1.0 → should admit >95%.
        let admitted = (0..1000).filter(|_| ac.should_admit(root)).count();
        assert!(
            admitted > 950,
            "zero ER should admit >95%, got {admitted}/1000"
        );
    }

    /// Helper: backdate the window clock so the next `record_outcome` call
    /// crosses the 50 ms boundary and fires the AIMD step.
    fn backdate_window(ac: &PredictiveAdmission, root_id: MethodId) {
        let mut states = ac.states_by_root.lock().unwrap();
        let state = states
            .entry(root_id)
            .or_insert_with(|| AdmissionState::new(Instant::now()));
        state.last_update = Instant::now() - std::time::Duration::from_millis(60);
    }

    fn admit_p(ac: &PredictiveAdmission, root_id: MethodId) -> f64 {
        ac.states_by_root
            .lock()
            .unwrap()
            .get(&root_id)
            .map(|state| state.admit_p)
            .unwrap_or(1.0)
    }

    #[test]
    fn test_aimd_decreases_admit_p_on_overloaded_window() {
        let ac = PredictiveAdmission::new();
        let root = test_root("test_aimd_decreases_admit_p_on_overloaded_window");
        // Drive 10 all-ER windows (100% ER fraction > any reasonable threshold).
        for _ in 0..10 {
            backdate_window(&ac, root);
            ac.record_outcome(root, true);
        }
        assert!(
            admit_p(&ac, root) < 1.0,
            "admit_p should decrease after overloaded windows, got {}",
            admit_p(&ac, root)
        );
    }

    #[test]
    fn test_aimd_increases_admit_p_on_healthy_window() {
        let ac = PredictiveAdmission::new();
        let root = test_root("test_aimd_increases_admit_p_on_healthy_window");
        // Set admit_p low, then drive all-success windows.
        {
            let mut states = ac.states_by_root.lock().unwrap();
            let state = states
                .entry(root)
                .or_insert_with(|| AdmissionState::new(Instant::now()));
            state.admit_p = 0.5;
        }
        for _ in 0..5 {
            backdate_window(&ac, root);
            ac.record_outcome(root, false);
        }
        assert!(
            admit_p(&ac, root) > 0.5,
            "admit_p should increase after healthy windows, got {}",
            admit_p(&ac, root)
        );
    }

    #[test]
    fn test_admit_p_starts_fully_open() {
        let ac = PredictiveAdmission::new();
        let root = test_root("test_admit_p_starts_fully_open");
        assert_eq!(admit_p(&ac, root), 1.0, "admit_p should start at 1.0");
    }

    #[test]
    fn test_admission_state_is_per_root() {
        let ac = PredictiveAdmission::new();
        let root_a = test_root("test_admission_state_is_per_root_a");
        let root_b = test_root("test_admission_state_is_per_root_b");
        for _ in 0..5 {
            backdate_window(&ac, root_a);
            ac.record_outcome(root_a, true);
        }

        assert!(
            admit_p(&ac, root_a) < 1.0,
            "overloaded root should reduce admit_p"
        );
        assert_eq!(
            admit_p(&ac, root_b),
            1.0,
            "unseen root should remain fully open"
        );
    }

    /// Verify idle decay reopens admission when no outcomes arrive.
    ///
    /// Scenario: admit_p is driven low by overload, then traffic goes idle.
    /// reject_prob = (1 - admit_p) * exp(-idle / tau_er) should decay toward 0
    /// even though admit_p itself hasn't changed.
    #[test]
    fn test_idle_decay_reopens_admission() {
        let ac = PredictiveAdmission::new();
        let root = test_root("test_idle_decay_reopens_admission");

        // Set admit_p very low to force near-total rejection.
        {
            let mut global_state = ac.global_state.lock().unwrap();
            global_state.admit_p = 0.01;
            global_state.er_last_update = Instant::now();
        }
        {
            let mut states = ac.states_by_root.lock().unwrap();
            let state = states
                .entry(root)
                .or_insert_with(|| AdmissionState::new(Instant::now()));
            state.admit_p = 0.01;
            state.er_last_update = Instant::now();
        }

        let admitted_before = (0..1000).filter(|_| ac.should_admit(root)).count();
        assert!(
            admitted_before < 100,
            "low admit_p should cause frequent rejection, got {admitted_before}/1000"
        );

        // Simulate 3 × tau_er (= 6 s at default tau_er=2 s) of idle.
        // idle_decay = exp(-3) ≈ 0.05 → reject_prob ≈ 0.99 × 0.05 ≈ 0.05.
        {
            let mut global_state = ac.global_state.lock().unwrap();
            global_state.er_last_update = Instant::now() - std::time::Duration::from_secs(6);
        }
        {
            let mut states = ac.states_by_root.lock().unwrap();
            let state = states
                .entry(root)
                .or_insert_with(|| AdmissionState::new(Instant::now()));
            state.er_last_update = Instant::now() - std::time::Duration::from_secs(6);
        }

        let admitted_after = (0..1000).filter(|_| ac.should_admit(root)).count();
        assert!(
            admitted_after > 900,
            "idle decay should open admission after 3×tau_er, got {admitted_after}/1000"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Microbenchmarks for the BCF feasibility check (run with --ignored).
    //
    // Measures the per-call overhead of the time-decay-aware probabilistic
    // shed logic. Three cases:
    //   - cold_start: no observations on either side → fast return
    //   - fast_path:  raw mean fits, no decay/exp/rand
    //   - shed_path:  overshoot triggers the probabilistic shed (worst case)
    // ──────────────────────────────────────────────────────────────────────

    use crate::layer::est::estimator::DefaultLatencyEstimator;
    use crate::layer::est::state::LatencyEstimators;
    use std::time::Instant;

    fn build_estimators_with_observations(
        key: ParentToChildKey,
        child_us: u64,
        remaining_us: u64,
    ) -> LatencyEstimators<DefaultLatencyEstimator> {
        let est = LatencyEstimators::<DefaultLatencyEstimator>::new();
        // Seed both estimators with steady-state observations.
        for _ in 0..200 {
            est.track_child_wallclock(key, child_us);
            est.track_after_child_wallclock(key, remaining_us);
        }
        est
    }

    /// Replicates the new BCF check body in isolation, with the same gate
    /// structure as `before_child_rpc`. Returns true if the request would be
    /// shed (`Err`), false if admitted.
    #[inline(never)]
    fn bcf_check(
        est: &LatencyEstimators<DefaultLatencyEstimator>,
        key: ParentToChildKey,
        now: u64,
        deadline: u64,
    ) -> bool {
        let time_left = deadline.saturating_sub(now);
        if time_left == 0 {
            return true;
        }
        let child = est.child_wallclock_pack(key).unwrap_or_default();
        let remaining = est.est_after_child_wallclock(key, time_left);
        let remaining_last_obs = est.after_child_wallclock_last_obs(key);

        if child.mean == 0 && remaining.mean == 0 {
            return false;
        }
        let decay = {
            let child_decay = decay_factor(now, child.last_observation_us);
            let rem_decay = decay_factor(now, remaining_last_obs);
            child_decay.min(rem_decay)
        };
        let floor_finish_us =
            now as f64 + (child.floor as f64 + remaining.floor as f64) * decay;
        if floor_finish_us > deadline as f64 {
            return true;
        }
        let raw_finish = now + child.mean + remaining.mean;
        if raw_finish <= deadline {
            return false;
        }
        let decayed_finish = if decay < 1.0 {
            now + ((child.mean as f64 + remaining.mean as f64) * decay) as u64
        } else {
            raw_finish
        };
        if decayed_finish <= deadline {
            return false;
        }
        let overshoot = decayed_finish - deadline;
        let ratio = overshoot as f64 / time_left as f64;
        let shed_prob = 1.0 - fast_exp_neg(LAMBDA * ratio);
        rand::random::<f64>() < shed_prob
    }

    fn bench_key() -> ParentToChildKey {
        let reg = crate::MethodRegistry::global();
        let r = reg.get_or_register(CowGrpcMethod::new("BenchSvc", "Root"));
        let p = reg.get_or_register(CowGrpcMethod::new("BenchSvc", "Parent"));
        let c = reg.get_or_register(CowGrpcMethod::new("BenchSvc", "Child"));
        ParentToChildKey::root_rpc_method(r)
            .parent_rpc_method(p)
            .child_rpc_method(c)
    }

    fn run_bench(label: &str, iterations: u64, mut f: impl FnMut() -> bool) {
        let mut checksum: u64 = 0;
        let start = Instant::now();
        for _ in 0..iterations {
            checksum = checksum.wrapping_add(f() as u64);
        }
        let elapsed = start.elapsed();
        let ns_per = elapsed.as_nanos() / u128::from(iterations);
        println!(
            "bcf_bench case={} iterations={} ns_per_call={} checksum={}",
            label, iterations, ns_per, checksum
        );
    }

    /// Replicates the *previous* BCF check body (mean est_child + floor remaining,
    /// no decay, no probabilistic shed). Kept here to quantify the overhead delta
    /// of the redesign in apples-to-apples conditions.
    #[inline(never)]
    fn bcf_check_legacy(
        est: &LatencyEstimators<DefaultLatencyEstimator>,
        key: ParentToChildKey,
        now: u64,
        deadline: u64,
    ) -> bool {
        let time_left = deadline.saturating_sub(now);
        let remaining = est.est_after_child_wallclock(key, time_left);
        let est_child = est.est_child_wallclock(key).unwrap_or(0);
        now + est_child + remaining.floor > deadline
    }

    #[test]
    #[ignore = "microbenchmark; run with --ignored --nocapture"]
    fn bcf_overhead_microbenchmark() {
        let key = bench_key();
        let now = masa_core::time_now();
        let deadline_far = now + 100_000; // 100 ms budget — fits comfortably
        let deadline_tight = now + 5_000; // 5 ms budget — overshoots

        // Cold start: no observations, both maps empty for this key.
        let cold = LatencyEstimators::<DefaultLatencyEstimator>::new();
        run_bench("cold_start", 100_000, || {
            bcf_check(&cold, key, now, deadline_far)
        });

        // Fast path: raw mean fits, no decay, no exp, no rand.
        // Child=8ms, remaining=4ms, deadline 100ms in future → 12ms ≤ 100ms.
        let warm_fits = build_estimators_with_observations(key, 8_000, 4_000);
        run_bench("fast_path_admit", 100_000, || {
            bcf_check(&warm_fits, key, now, deadline_far)
        });

        // Shed path: child=8ms, remaining=4ms, but only 5ms time_left → 12ms > 5ms.
        // Triggers floor backstop and/or probabilistic shed.
        run_bench("shed_path", 100_000, || {
            bcf_check(&warm_fits, key, now, deadline_tight)
        });

        // Legacy comparison: same scenarios run through the old BCF body.
        run_bench("legacy_cold_start", 100_000, || {
            bcf_check_legacy(&cold, key, now, deadline_far)
        });
        run_bench("legacy_fast_path", 100_000, || {
            bcf_check_legacy(&warm_fits, key, now, deadline_far)
        });
        run_bench("legacy_shed_path", 100_000, || {
            bcf_check_legacy(&warm_fits, key, now, deadline_tight)
        });
    }

    #[test]
    #[ignore = "microbenchmark; run with --ignored --nocapture"]
    fn fast_exp_neg_vs_libm() {
        // Compare our Padé approximant against f64::exp on a representative range.
        let inputs: Vec<f64> = (0..1000).map(|i| (i as f64) * 0.005).collect(); // 0..5
        let mut checksum = 0.0_f64;

        let start = Instant::now();
        for _ in 0..1000 {
            for &x in &inputs {
                checksum += fast_exp_neg(x);
            }
        }
        let fast_elapsed = start.elapsed();

        let mut checksum_libm = 0.0_f64;
        let start = Instant::now();
        for _ in 0..1000 {
            for &x in &inputs {
                checksum_libm += (-x).exp();
            }
        }
        let libm_elapsed = start.elapsed();

        let n = 1000 * inputs.len();
        println!(
            "exp_bench fast_exp_neg ns_per_call={} checksum={:.4}",
            fast_elapsed.as_nanos() / n as u128,
            checksum
        );
        println!(
            "exp_bench f64::exp     ns_per_call={} checksum={:.4}",
            libm_elapsed.as_nanos() / n as u128,
            checksum_libm
        );
        // Worst-case relative error report.
        let mut max_rel_err = 0.0_f64;
        for &x in &inputs {
            let approx = fast_exp_neg(x);
            let exact = (-x).exp();
            let rel = (approx - exact).abs() / exact.max(1e-12);
            if rel > max_rel_err {
                max_rel_err = rel;
            }
        }
        println!(
            "exp_bench fast_exp_neg max_rel_err on [0,5] = {:.4e}",
            max_rel_err
        );
    }
}
