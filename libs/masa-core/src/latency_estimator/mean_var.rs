use super::LatencyEstimator;
use serde::{Deserialize, Serialize};

/// Running mean + k*stddev latency estimator.
///
/// The estimate is computed as:
///   mean + k * stddev
/// where mean and variance are maintained online using Welford's algorithm,
/// and stddev = sqrt(variance). This is a standard sigma-bound estimator.
///
/// Updates use a doubling schedule during warmup: the estimate is recomputed
/// after observations 1, 2, 4, 8, ..., stabilizing to `update_interval` once
/// sufficient data exists. This avoids the step-function jump from 0 to the
/// full estimate after `update_interval` cold observations.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LatencyMeanVar {
    count: usize,
    mean: f64,
    m2: f64,
    k: f64,
    estimate: u64,
    update_interval: usize,
    /// Absolute count at which the next estimate update will occur.
    next_update: usize,
}

impl LatencyMeanVar {
    pub fn new(k: f64, update_interval: usize) -> Self {
        assert!(k >= 0.0, "k should be non-negative");
        assert!(update_interval >= 1, "Update interval should be at least 1");
        Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
            k,
            estimate: 0,
            update_interval,
            next_update: 1,
        }
    }

    fn update_estimate(&mut self) {
        if self.count == 0 {
            self.estimate = 0;
            return;
        }

        let variance = self.m2 / self.count as f64;
        let raw_estimate = self.mean + self.k * variance.sqrt();
        self.estimate = if raw_estimate.is_finite() && raw_estimate > 0.0 {
            raw_estimate.min(u64::MAX as f64) as u64
        } else {
            0
        };
    }

    pub fn mean(&self) -> f64 {
        self.mean
    }

    pub fn variance(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.m2 / self.count as f64
        }
    }
}

impl LatencyEstimator for LatencyMeanVar {
    fn track(&mut self, value: u64) {
        let x = value as f64;
        self.count += 1;

        // Welford online update for stable running mean/variance.
        let delta = x - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = x - self.mean;
        self.m2 += delta * delta2;

        if self.count >= self.next_update {
            self.update_estimate();
            // Doubling schedule during warmup; then fixed interval thereafter.
            if self.next_update < self.update_interval {
                self.next_update = (self.next_update * 2).min(self.update_interval);
            } else {
                self.next_update = self.count + self.update_interval;
            }
        }
    }

    fn can_estimate(&self) -> bool {
        self.count > 0
    }

    fn estimate(&self) -> u64 {
        self.estimate
    }
}

impl Default for LatencyMeanVar {
    fn default() -> Self {
        // k=1.0 and periodic updates to match the behavior of other estimators.
        Self::new(1.0, 512)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let est = LatencyMeanVar::new(2.0, 16);
        assert_eq!(est.count, 0);
        assert_eq!(est.mean, 0.0);
        assert_eq!(est.m2, 0.0);
        assert_eq!(est.k, 2.0);
        assert_eq!(est.estimate, 0);
        assert_eq!(est.update_interval, 16);
        assert_eq!(est.next_update, 1);
    }

    #[test]
    #[should_panic(expected = "k should be non-negative")]
    fn test_new_invalid_k() {
        LatencyMeanVar::new(-1.0, 1);
    }

    #[test]
    #[should_panic(expected = "Update interval should be at least 1")]
    fn test_new_invalid_interval() {
        LatencyMeanVar::new(1.0, 0);
    }

    #[test]
    fn test_default() {
        let est = LatencyMeanVar::default();
        assert_eq!(est.k, 1.0);
        assert_eq!(est.update_interval, 512);
        assert_eq!(est.next_update, 1);
    }

    #[test]
    fn test_mean_and_variance() {
        let mut est = LatencyMeanVar::new(0.0, 1);
        est.track(10);
        est.track(20);
        est.track(30);

        // Values: [10, 20, 30], mean=20, population variance=66.66...
        assert!((est.mean() - 20.0).abs() < 1e-6);
        assert!((est.variance() - (200.0 / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn test_estimate_mean_plus_k_stddev() {
        let mut est = LatencyMeanVar::new(1.0, 1);
        est.track(10);
        est.track(20);
        est.track(30);

        // Values: [10, 20, 30], mean=20, population variance=200/3≈66.67, stddev≈8.165
        // estimate = mean + k * stddev = 20 + 8.165 ≈ 28
        let v = est.estimate();
        assert!(v >= 28 && v <= 29);
    }

    #[test]
    fn test_doubling_warmup_schedule() {
        // With update_interval=16, updates at count: 1, 2, 4, 8, 16, 32, 48, ...
        let mut est = LatencyMeanVar::new(0.0, 16);

        // Before any tracks, estimate is 0
        assert_eq!(est.estimate(), 0);

        // After first observation, estimate updates immediately (count >= next_update=1)
        est.track(100);
        assert_eq!(est.estimate(), 100); // mean=100, k=0 so estimate=mean=100

        // After second observation, estimate updates (next_update was set to 2)
        est.track(200);
        assert_eq!(est.estimate(), 150); // mean=150

        // After 4th observation, estimate updates (next_update was set to 4)
        est.track(300);
        // count=3, next_update=4, no update yet
        assert_eq!(est.estimate(), 150);

        est.track(400);
        // count=4, next_update=4 → update. mean = (100+200+300+400)/4 = 250
        assert_eq!(est.estimate(), 250);

        // After 8th observation
        est.track(500);
        est.track(600);
        est.track(700);
        // count=7, next_update=8, no update
        assert_eq!(est.estimate(), 250);

        est.track(800);
        // count=8, next_update=8 → update. mean = (100..800)/8 = 450
        assert_eq!(est.estimate(), 450);
    }

    #[test]
    fn test_steady_state_interval() {
        // After warmup completes, should update every update_interval observations
        let mut est = LatencyMeanVar::new(0.0, 4);

        // Drive through warmup (updates at 1, 2, 4)
        for i in 1..=4 {
            est.track(i * 100);
        }
        // count=4, warmup done (next_update was set to count + update_interval = 4+4=8)
        let est_after_warmup = est.estimate(); // mean of [100,200,300,400] = 250
        assert_eq!(est_after_warmup, 250);

        // Track 3 more - no update
        est.track(100);
        est.track(100);
        est.track(100);
        assert_eq!(est.estimate(), 250);

        // Track 4th more (count=8) - update happens
        est.track(100);
        // new mean = (100+200+300+400+100+100+100+100)/8 = 1500/8 = 187.5 → 187
        assert!(est.estimate() < 250);
    }

    #[test]
    fn test_can_estimate() {
        let mut est = LatencyMeanVar::new(1.0, 10);
        assert!(!est.can_estimate());
        est.track(1);
        assert!(est.can_estimate());
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut est = LatencyMeanVar::new(2.0, 1);
        est.track(10);
        est.track(20);

        let serialized = serde_json::to_string(&est).unwrap();
        let deserialized: LatencyMeanVar = serde_json::from_str(&serialized).unwrap();
        assert_eq!(est.count, deserialized.count);
        assert_eq!(est.k, deserialized.k);
        assert_eq!(est.update_interval, deserialized.update_interval);
        assert_eq!(est.next_update, deserialized.next_update);
    }
}
