// Zero-cost wrapper for est-based admission control.
//
// When `ac_est` is enabled, PredictiveAc holds an AdmissionController and
// reads estimation maps from EstServerState to make admission decisions.
// When disabled, all methods are no-ops that compile away entirely.
//
// Dependency direction: this module reads from `est/state` (estimation maps).
// The estimation module has no knowledge of admission control.

use masa_core::Context;

use super::super::est::estimator::DefaultLatencyEstimator;
use super::super::est::state::EstServerState;

/// Zero-cost admission control overlay using latency estimates.
///
/// This follows the same dual-impl pattern as `PredictiveAbort`: a single
/// struct with feature-gated implementations. `standard.rs` calls methods
/// unconditionally; the compiler eliminates no-op paths entirely.

// ── ac_est ENABLED ───────────────────────────────────────────────────────────

#[cfg(feature = "ac_est")]
use super::est::AdmissionController;

#[cfg(feature = "ac_est")]
#[derive(Debug)]
pub(crate) struct PredictiveAc {
    controller: AdmissionController,
}

#[cfg(feature = "ac_est")]
impl PredictiveAc {
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
        use masa_core::{time_now, SLO_ABORT};

        if !SLO_ABORT {
            return false;
        }

        let time_left = ctx.e2e_deadline().saturating_sub(time_now());

        // Layer 1: compute-time feasibility (every hop)
        let est_compute_rem = est_server
            .est_compute_latency
            .get_estimate(resolved_method_id)
            .unwrap_or(0);
        if est_compute_rem > time_left {
            return true; // infeasible → shed
        }

        // Layer 2: efficiency-based admission (ingress only)
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

// ── ac_est DISABLED ──────────────────────────────────────────────────────────

#[cfg(not(feature = "ac_est"))]
#[derive(Debug)]
pub(crate) struct PredictiveAc;

#[cfg(not(feature = "ac_est"))]
impl PredictiveAc {
    #[inline]
    pub(crate) fn new() -> Self {
        Self
    }

    #[inline]
    pub(crate) fn admission_check(
        &self,
        _est_server: &EstServerState<DefaultLatencyEstimator>,
        _resolved_method_id: u64,
        _ctx: &Context,
        _key: u64,
    ) -> bool {
        false
    }

    #[inline]
    pub(crate) fn update_bottleneck(&self, _api: &str, _max_downstream_util: f32) {}
}
