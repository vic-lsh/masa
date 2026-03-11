use super::LatencyEstimator;
use serde::{Deserialize, Serialize};

/// Exponential Moving Average (EMA) latency estimator: mean + k*stddev.
///
/// Uses asymmetric EMA: alpha_up=0.05 when a new observation exceeds the current
/// mean (slow to inflate), alpha_down=0.2 when at or below (fast to deflate).
/// This prevents transient spikes from inflating estimates while ensuring rapid
/// adaptation when load drops, avoiding stale over-estimates that cause unnecessary
/// early returns.
///
/// The estimate is: mean + k * stddev, where stddev = sqrt(EMA variance).
/// Updated on every tracked observation (no batching).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LatencyMeanVar {
    mean: f64,
    variance: f64,
    k: f64,
    alpha: f64,
    estimate: u64,
    initialized: bool,
    /// Floor estimate: slow-to-inflate (α=0.01), fast-to-deflate (α=0.3) EMA.
    /// Tracks the lower envelope of latency observations to provide a conservative
    /// lower bound that is resistant to mean inflation during load spikes.
    mean_floor: f64,
}

impl LatencyMeanVar {
    pub fn new(k: f64, alpha: f64) -> Self {
        assert!(k >= 0.0, "k should be non-negative");
        assert!(alpha > 0.0 && alpha <= 1.0, "alpha must be in (0, 1]");
        Self {
            mean: 0.0,
            variance: 0.0,
            k,
            alpha,
            estimate: 0,
            initialized: false,
            mean_floor: 0.0,
        }
    }

    pub fn mean(&self) -> f64 {
        self.mean
    }

    pub fn variance(&self) -> f64 {
        self.variance
    }
}

impl LatencyEstimator for LatencyMeanVar {
    fn track(&mut self, value: u64) {
        let x = value as f64;
        if !self.initialized {
            self.mean = x;
            self.variance = 0.0;
            self.mean_floor = x;
            self.initialized = true;
        } else {
            // Asymmetric alpha: slow to inflate (observations above mean), fast to deflate.
            // alpha_up=0.05 prevents latency spikes from inflating estimates too quickly.
            // alpha_down=0.2 ensures rapid adaptation when load drops, avoiding stale
            // over-estimates that cause unnecessary early returns.
            let alpha = if x > self.mean { 0.05 } else { 0.2 };
            let delta = x - self.mean;
            self.mean += alpha * delta;
            self.variance = (1.0 - alpha) * (self.variance + alpha * delta * delta);

            // Floor EMA: fast deflation (α=0.3) when observation is below floor,
            // very slow inflation (α=0.01) when above. Tracks the lower envelope of
            // latency to provide a conservative ER threshold resistant to mean inflation.
            let alpha_floor = if x < self.mean_floor { 0.3 } else { 0.01 };
            self.mean_floor += alpha_floor * (x - self.mean_floor);
        }
        let raw = self.mean + self.k * self.variance.sqrt();
        self.estimate = if raw.is_finite() && raw > 0.0 {
            raw.min(u64::MAX as f64) as u64
        } else {
            0
        };
    }

    fn can_estimate(&self) -> bool {
        self.initialized
    }

    fn estimate(&self) -> u64 {
        self.estimate
    }

    fn mean_estimate(&self) -> u64 {
        if self.mean > 0.0 && self.mean.is_finite() {
            self.mean.min(u64::MAX as f64) as u64
        } else {
            0
        }
    }

    fn mean_floor_estimate(&self) -> u64 {
        if self.mean_floor > 0.0 && self.mean_floor.is_finite() {
            self.mean_floor.min(u64::MAX as f64) as u64
        } else {
            0
        }
    }
}

impl Default for LatencyMeanVar {
    fn default() -> Self {
        // k=0.0: pure mean estimator — variance term removed (PINE Iteration 2).
        // alpha field is kept for API compatibility but the EMA update uses hardcoded
        // asymmetric values: alpha_up=0.05 (slow inflation) and alpha_down=0.2 (fast
        // deflation). See track() for rationale.
        Self::new(0.0, 0.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let est = LatencyMeanVar::new(2.0, 0.1);
        assert_eq!(est.k, 2.0);
        assert_eq!(est.alpha, 0.1);
        assert!(!est.initialized);
        assert_eq!(est.estimate, 0);
    }

    #[test]
    #[should_panic(expected = "k should be non-negative")]
    fn test_new_invalid_k() {
        LatencyMeanVar::new(-1.0, 0.1);
    }

    #[test]
    #[should_panic(expected = "alpha must be in (0, 1]")]
    fn test_new_invalid_alpha_zero() {
        LatencyMeanVar::new(1.0, 0.0);
    }

    #[test]
    #[should_panic(expected = "alpha must be in (0, 1]")]
    fn test_new_invalid_alpha_above_one() {
        LatencyMeanVar::new(1.0, 1.1);
    }

    #[test]
    fn test_default() {
        let est = LatencyMeanVar::default();
        assert_eq!(est.k, 0.0);
        assert_eq!(est.alpha, 0.1);
        assert!(!est.initialized);
    }

    #[test]
    fn test_can_estimate() {
        let mut est = LatencyMeanVar::new(1.0, 0.1);
        assert!(!est.can_estimate());
        est.track(100);
        assert!(est.can_estimate());
    }

    #[test]
    fn test_first_observation_initializes_mean() {
        let mut est = LatencyMeanVar::new(0.0, 0.5);
        est.track(1000);
        // First observation: mean = 1000, variance = 0, estimate = mean + 0*0 = 1000
        assert_eq!(est.mean(), 1000.0);
        assert_eq!(est.variance(), 0.0);
        assert_eq!(est.estimate(), 1000);
    }

    #[test]
    fn test_mean_convergence_constant_input() {
        // After many equal values, mean should converge to that value
        let mut est = LatencyMeanVar::new(0.0, 0.1);
        for _ in 0..100 {
            est.track(500);
        }
        assert!((est.mean() - 500.0).abs() < 1.0);
        assert_eq!(est.estimate(), 500);
    }

    #[test]
    fn test_mean_adapts_to_step_change() {
        // After 50 observations at 100, then switch to 1000.
        // With alpha=0.2, mean should approach 1000 within ~30 additional obs.
        let mut est = LatencyMeanVar::new(0.0, 0.2);
        for _ in 0..50 {
            est.track(100);
        }
        assert!((est.mean() - 100.0).abs() < 1.0);

        for _ in 0..30 {
            est.track(1000);
        }
        // After 30 more obs at 1000 with alpha=0.2, mean should be well above 500
        assert!(
            est.mean() > 800.0,
            "mean should have shifted toward 1000, got {}",
            est.mean()
        );
    }

    #[test]
    fn test_estimate_is_mean_plus_k_stddev() {
        // With constant input, variance → 0 and estimate → mean
        let mut est = LatencyMeanVar::new(1.0, 0.1);
        for _ in 0..100 {
            est.track(300);
        }
        // estimate ≈ 300 + 1.0 * ~0 ≈ 300
        assert!((est.estimate() as f64 - 300.0).abs() < 2.0);
    }

    #[test]
    fn test_mean_estimate_returns_mean() {
        let mut est = LatencyMeanVar::new(2.0, 0.1);
        for _ in 0..50 {
            est.track(400);
        }
        let mean_est = est.mean_estimate() as f64;
        assert!((mean_est - est.mean()).abs() < 1.0);
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut est = LatencyMeanVar::new(2.0, 0.1);
        est.track(100);
        est.track(200);

        let serialized = serde_json::to_string(&est).unwrap();
        let deserialized: LatencyMeanVar = serde_json::from_str(&serialized).unwrap();
        assert_eq!(est.k, deserialized.k);
        assert_eq!(est.alpha, deserialized.alpha);
        assert_eq!(est.initialized, deserialized.initialized);
        assert!((est.mean() - deserialized.mean()).abs() < 1e-10);
    }
}
