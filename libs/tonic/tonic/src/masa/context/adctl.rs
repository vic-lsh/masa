use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

const STALENESS_SECS: f64 = 2.0;
const STALENESS_DEFAULT: f32 = 0.5;
const UTIL_TARGET: f64 = 0.85;
const ADJUST_RATE: f64 = 0.5;
const MAX_BURST_SECS: f64 = 0.1;
const INITIAL_BUDGET_RATE: f64 = 10_000_000.0; // µs/s — start generous

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

struct BudgetState {
    budget_us: f64,
    budget_rate: f64,
    last_refill: Instant,
}

/// Admission controller using compute-budget token bucket.
#[derive(Debug)]
pub(crate) struct AdmissionController {
    bottleneck: BottleneckTracker,
    state: Mutex<BudgetState>,
}

// BudgetState doesn't implement Debug, so we need a manual impl
impl std::fmt::Debug for BudgetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BudgetState")
            .field("budget_us", &self.budget_us)
            .field("budget_rate", &self.budget_rate)
            .finish()
    }
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
        // Initial budget = INITIAL_BUDGET_RATE * MAX_BURST_SECS = 10M * 0.1 = 1M µs
        // Each request costs 100_000 µs, so ~10 requests should exhaust it
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
