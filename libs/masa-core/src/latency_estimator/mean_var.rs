use super::LatencyEstimator;
use serde::{Deserialize, Serialize};

/// Running mean + k*stddev latency estimator.
///
/// The estimate is computed as:
///   mean + k * stddev
/// where mean and variance are maintained online using Welford's algorithm,
/// and stddev = sqrt(variance). This is a standard sigma-bound estimator.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LatencyMeanVar {
    count: usize,
    mean: f64,
    m2: f64,
    k: f64,
    estimate: u64,
    update_interval: usize,
    since_last_update: usize,
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
            since_last_update: 0,
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
        self.since_last_update += 1;

        // Welford online update for stable running mean/variance.
        let delta = x - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = x - self.mean;
        self.m2 += delta * delta2;

        if self.since_last_update >= self.update_interval {
            self.update_estimate();
            self.since_last_update = 0;
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
        assert_eq!(est.since_last_update, 0);
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
    fn test_update_interval() {
        let mut est = LatencyMeanVar::new(1.0, 3);
        est.track(10);
        est.track(20);
        assert_eq!(est.estimate(), 0);

        est.track(30);
        assert!(est.estimate() > 0);
        assert_eq!(est.since_last_update, 0);
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
    }
}
