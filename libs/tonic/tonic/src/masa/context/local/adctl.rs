use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

const STALENESS_SECS: f64 = 2.0;
const STALENESS_DEFAULT: f32 = 0.5;
const UTIL_TARGET: f64 = 0.85;
const RAISE_RATE: f64 = 1.0; // threshold units per second when overloaded
const HALF_LIFE_SECS: f64 = 1.0; // threshold half-life during decay

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

const REFERENCE_COMPUTE: f64 = 50_000.0; // 50ms in microseconds

fn efficiency_score(p_feasible: f64, est_compute: u64) -> f64 {
    p_feasible / (1.0 + est_compute as f64 / REFERENCE_COMPUTE)
}

/// Admission controller using efficiency-based threshold feedback.
#[derive(Debug)]
pub(crate) struct AdmissionController {
    bottleneck: BottleneckTracker,
    threshold: Mutex<f64>,
    last_update: Mutex<Instant>,
}

impl AdmissionController {
    pub(crate) fn new() -> Self {
        Self {
            bottleneck: BottleneckTracker::new(),
            threshold: Mutex::new(0.0),
            last_update: Mutex::new(Instant::now()),
        }
    }

    pub(crate) fn update_bottleneck(&self, api: &str, max_downstream_util: f32) {
        self.bottleneck.update(api, max_downstream_util);
    }

    /// Returns true if the request should be admitted.
    pub(crate) fn should_admit(
        &self,
        api: &str,
        time_left: u64,
        est_compute: u64,
        est_total_mean: u64,
    ) -> bool {
        let p_feasible = if time_left == 0 {
            0.0
        } else {
            (1.0 - est_total_mean as f64 / time_left as f64).max(0.0)
        };

        let score = efficiency_score(p_feasible, est_compute);

        let now = std::time::Instant::now();
        let mut last = self.last_update.lock().unwrap();
        let elapsed_secs = now.duration_since(*last).as_secs_f64();
        *last = now;
        drop(last);

        let bottleneck_util = self.bottleneck.get(api) as f64;
        let mut threshold = self.threshold.lock().unwrap();
        if bottleneck_util > UTIL_TARGET {
            *threshold += RAISE_RATE * elapsed_secs;
        } else {
            *threshold *= 0.5_f64.powf(elapsed_secs / HALF_LIFE_SECS);
        }
        *threshold = threshold.clamp(0.0, 1.0);
        score >= *threshold
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
    fn test_efficiency_score() {
        // 0.5 / (1 + 100/50000) = 0.5 / 1.002 ≈ 0.499001996
        assert!((efficiency_score(0.5, 100) - 0.499001996).abs() < 1e-6);
        // 1.0 / (1 + 0/50000) = 1.0
        assert!((efficiency_score(1.0, 0) - 1.0).abs() < 1e-6);
        assert!((efficiency_score(0.0, 100) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_admission_controller_admits_with_budget() {
        let ac = AdmissionController::new();
        // With lots of time left and low compute, should admit
        let admitted = ac.should_admit("Search", 100_000, 1000, 50_000);
        assert!(admitted);
    }

    #[test]
    fn test_admission_controller_rejects_infeasible() {
        let ac = AdmissionController::new();
        // Simulate high utilization to raise threshold
        ac.update_bottleneck("Search", 0.95);
        // Backdate last_update by 1 second so RAISE_RATE * 1.0s = 1.0 threshold
        *ac.last_update.lock().unwrap() = Instant::now() - std::time::Duration::from_secs(1);
        // One call raises threshold to 1.0
        ac.should_admit("Search", 100_000, 1000, 50_000);
        // Now with est_total_mean >= time_left → p_feasible=0 → score=0
        let admitted = ac.should_admit("Search", 1000, 100, 2000);
        assert!(!admitted);
    }
}
