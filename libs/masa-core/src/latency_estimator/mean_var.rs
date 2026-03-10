use super::LatencyEstimator;
use serde::{Deserialize, Serialize};

/// Exponential Moving Average (EMA) latency estimator: mean + k*stddev.
///
/// Uses asymmetric EMA: `alpha_up` when the new observation exceeds the current mean
/// (latency increasing), and `alpha` when it is at or below the mean (latency stable
/// or decreasing, including 0-injections from early-return feedback suppression).
///
/// The estimate is: mean + k * stddev, where stddev = sqrt(EMA variance).
/// Updated on every tracked observation (no batching).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LatencyMeanVar {
    mean: f64,
    variance: f64,
    k: f64,
    alpha: f64,
    alpha_up: f64,
    estimate: u64,
    initialized: bool,
}

impl LatencyMeanVar {
    /// Create a symmetric estimator (alpha_up == alpha).
    pub fn new(k: f64, alpha: f64) -> Self {
        Self::new_asymmetric(k, alpha, alpha)
    }

    /// Create an asymmetric estimator with separate adaptation rates for
    /// latency increases (`alpha_up`) and decreases (`alpha`).
    pub fn new_asymmetric(k: f64, alpha: f64, alpha_up: f64) -> Self {
        assert!(k >= 0.0, "k should be non-negative");
        assert!(alpha > 0.0 && alpha <= 1.0, "alpha must be in (0, 1]");
        assert!(
            alpha_up > 0.0 && alpha_up <= 1.0,
            "alpha_up must be in (0, 1]"
        );
        Self {
            mean: 0.0,
            variance: 0.0,
            k,
            alpha,
            alpha_up,
            estimate: 0,
            initialized: false,
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
            self.initialized = true;
        } else {
            let delta = x - self.mean;
            // Use alpha_up for latency increases; alpha for decreases (incl. 0-injections).
            let alpha = if delta > 0.0 {
                self.alpha_up
            } else {
                self.alpha
            };
            self.mean += alpha * delta;
            self.variance = (1.0 - alpha) * (self.variance + alpha * delta * delta);
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
}

impl Default for LatencyMeanVar {
    fn default() -> Self {
        // k=1.0: estimate at ~84th percentile.
        // alpha=0.1: effective window ~10 obs for latency decreases / 0-injections.
        // alpha_up=0.2: effective window ~5 obs for latency increases (faster reaction
        // to overload onset while preserving stable 0-injection feedback suppression).
        Self::new_asymmetric(1.0, 0.1, 0.2)
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
        assert_eq!(est.alpha_up, 0.1);
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
        assert_eq!(est.k, 1.0);
        assert_eq!(est.alpha, 0.1);
        assert_eq!(est.alpha_up, 0.2);
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
        // With alpha_up=0.2, mean should approach 1000 within ~30 additional obs.
        let mut est = LatencyMeanVar::new(0.0, 0.2);
        for _ in 0..50 {
            est.track(100);
        }
        assert!((est.mean() - 100.0).abs() < 1.0);

        for _ in 0..30 {
            est.track(1000);
        }
        // After 30 more obs at 1000 with alpha_up=0.2, mean should be well above 500
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
    fn test_asymmetric_adapts_faster_up_than_down() {
        // With alpha_up=0.3, alpha_down=0.05: a step up should converge faster than a step down.
        let mut est = LatencyMeanVar::new_asymmetric(0.0, 0.05, 0.3);
        for _ in 0..100 {
            est.track(100);
        }
        let mean_after_low = est.mean();
        assert!((mean_after_low - 100.0).abs() < 1.0);

        // Step up: 10 observations at 1000
        for _ in 0..10 {
            est.track(1000);
        }
        let mean_after_up = est.mean();

        // Step down back to 100: 10 observations
        let mean_before_down = mean_after_up;
        for _ in 0..10 {
            est.track(100);
        }
        let mean_after_down = est.mean();

        // After 10 up-steps (alpha_up=0.3), mean should have moved more toward 1000
        // than after 10 down-steps (alpha=0.05) it moved back toward 100.
        let up_delta = mean_after_up - mean_after_low;
        let down_delta = mean_before_down - mean_after_down;
        assert!(
            up_delta > down_delta,
            "up_delta={:.1} should exceed down_delta={:.1}",
            up_delta,
            down_delta
        );
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
        assert_eq!(est.alpha_up, deserialized.alpha_up);
        assert_eq!(est.initialized, deserialized.initialized);
        assert!((est.mean() - deserialized.mean()).abs() < 1e-10);
    }
}
