// Predictive admission control.
//
// Provides `PredictiveAdmission` (zero-cost wrapper selecting between full ac_pred
// and floor-only checks), `AdmissionController` (compute-budget token bucket),
// and `BottleneckTracker` (per-API utilization tracking with staleness decay).
//
// `PredictiveAdmission` is owned by `PredictiveOverlay` and called from
// `before_child_rpc`. It reads estimation maps from `EstServerState`.

use masa_core::Context;

use super::est::estimator::DefaultLatencyEstimator;
use super::est::state::EstServerState;

// ── Bottleneck Tracker ──────────────────────────────────────────────────

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

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
