use super::LatencyEstimator;
use serde::{Deserialize, Serialize};

/// Exponential Weighted Moving Average (EWMA) latency estimator.
/// Recent samples are weighted more heavily, naturally forgetting old data.
///
/// α = alpha_num / alpha_den. Higher α reacts faster but is noisier.
/// The default (α = 1/8) has a half-life of ~5 samples.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LatencyEwma {
    alpha_num: u64,
    alpha_den: u64,
    ewma: u64,
    has_sample: bool,
}

impl LatencyEwma {
    pub fn new(alpha_num: u64, alpha_den: u64) -> Self {
        assert!(
            alpha_num > 0 && alpha_num < alpha_den,
            "alpha must satisfy 0 < alpha_num < alpha_den"
        );
        LatencyEwma {
            alpha_num,
            alpha_den,
            ewma: 0,
            has_sample: false,
        }
    }
}

impl LatencyEstimator for LatencyEwma {
    fn track(&mut self, value: u64) {
        if !self.has_sample {
            self.ewma = value;
            self.has_sample = true;
        } else {
            // ewma = alpha * value + (1 - alpha) * ewma
            // Use u128 for intermediates to avoid overflow.
            let new_ewma = (self.alpha_num as u128 * value as u128
                + (self.alpha_den - self.alpha_num) as u128 * self.ewma as u128)
                / self.alpha_den as u128;
            self.ewma = new_ewma as u64;
        }
    }

    fn can_estimate(&self) -> bool {
        self.has_sample
    }

    fn estimate(&self) -> u64 {
        self.ewma
    }
}

impl Default for LatencyEwma {
    fn default() -> Self {
        // α = 1/8 = 0.125: half-life of ~5 samples
        LatencyEwma::new(1, 8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_sample_initializes_ewma() {
        let mut e = LatencyEwma::new(1, 4);
        assert!(!e.can_estimate());
        e.track(100);
        assert!(e.can_estimate());
        assert_eq!(e.estimate(), 100);
    }

    #[test]
    fn test_ewma_weighted_update() {
        // α = 1/2 for easy hand-calculation
        let mut e = LatencyEwma::new(1, 2);
        e.track(100);
        e.track(200);
        // ewma = 0.5*200 + 0.5*100 = 150
        assert_eq!(e.estimate(), 150);
    }

    #[test]
    fn test_converges_to_constant() {
        let mut e = LatencyEwma::default();
        for _ in 0..100 {
            e.track(1000);
        }
        assert_eq!(e.estimate(), 1000);
    }

    #[test]
    fn test_adapts_to_step_change() {
        let mut e = LatencyEwma::new(1, 2);
        for _ in 0..20 {
            e.track(0);
        }
        assert_eq!(e.estimate(), 0);
        e.track(10000);
        // After one sample at 10000 with α=0.5: ewma = 5000
        assert_eq!(e.estimate(), 5000);
    }

    #[test]
    #[should_panic(expected = "alpha must satisfy")]
    fn test_invalid_alpha_zero() {
        LatencyEwma::new(0, 4);
    }

    #[test]
    #[should_panic(expected = "alpha must satisfy")]
    fn test_invalid_alpha_one() {
        LatencyEwma::new(4, 4);
    }

    #[test]
    fn test_default_alpha() {
        let e = LatencyEwma::default();
        assert_eq!(e.alpha_num, 1);
        assert_eq!(e.alpha_den, 8);
    }

    #[test]
    fn test_no_overflow_large_values() {
        let mut e = LatencyEwma::default();
        // u64::MAX / 2 to stay within range
        e.track(u64::MAX / 2);
        e.track(u64::MAX / 2);
        assert!(e.can_estimate());
    }

    #[test]
    fn test_decays_old_data() {
        // Confirm that old high values are forgotten after enough new low samples
        let mut e = LatencyEwma::new(1, 2);
        for _ in 0..20 {
            e.track(10000);
        }
        for _ in 0..20 {
            e.track(0);
        }
        // After 20 samples of 0 with α=0.5, the old 10000 is negligible
        assert!(e.estimate() < 10);
    }
}
