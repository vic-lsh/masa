use super::LatencyEstimator;
use serde::{Deserialize, Serialize};

/// Root Mean Square (RMS) based latency estimator.
/// This estimator calculates the RMS of observed latency values, which provides
/// a measure that accounts for variance in the latency distribution.
/// Uses incremental counters to track sum(x^2) and count, updating RMS periodically
/// to amortize the cost of the square root operation.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LatencyRms {
    sum_squares: u128,
    count: usize,
    rms: u64,
    update_interval: usize,
    since_last_update: usize,
}

impl LatencyRms {
    pub fn new(update_interval: usize) -> Self {
        assert!(update_interval >= 1, "Update interval should be at least 1");
        LatencyRms {
            sum_squares: 0,
            count: 0,
            rms: 0,
            update_interval,
            since_last_update: 0,
        }
    }

    fn update_rms(&mut self) {
        if self.count == 0 {
            self.rms = 0;
            return;
        }

        // Calculate RMS: sqrt(sum(x^2) / n)
        let mean_square = self.sum_squares / self.count as u128;
        // Use integer square root approximation
        self.rms = integer_sqrt(mean_square) as u64;
    }

    pub fn rms(&self) -> u64 {
        self.rms
    }
}

impl LatencyEstimator for LatencyRms {
    fn track(&mut self, value: u64) {
        // Incrementally update sum of squares and count
        let value_squared = (value as u128) * (value as u128);
        self.sum_squares += value_squared;
        self.count += 1;
        self.since_last_update += 1;

        // Periodically update RMS to amortize sqrt cost
        if self.since_last_update >= self.update_interval {
            self.update_rms();
            self.since_last_update = 0;
        }
    }

    fn can_estimate(&self) -> bool {
        self.count > 0
    }

    fn estimate(&self) -> u64 {
        // RMS ignores percentile parameter since it's a single value
        // Just return the cached RMS value
        self.rms
    }
}

impl Default for LatencyRms {
    fn default() -> Self {
        // Use a reasonable default update interval (every 512 track calls)
        LatencyRms::new(512)
    }
}

/// Integer square root using Newton's method
fn integer_sqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    if n < 4 {
        return 1;
    }

    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let rms = LatencyRms::new(100);
        assert_eq!(rms.sum_squares, 0);
        assert_eq!(rms.count, 0);
        assert_eq!(rms.rms, 0);
        assert_eq!(rms.update_interval, 100);
        assert_eq!(rms.since_last_update, 0);
    }

    #[test]
    #[should_panic(expected = "Update interval should be at least 1")]
    fn test_new_invalid_interval() {
        LatencyRms::new(0);
    }

    #[test]
    fn test_default() {
        let rms = LatencyRms::default();
        assert_eq!(rms.update_interval, 512);
        assert_eq!(rms.count, 0);
        assert_eq!(rms.rms, 0);
    }

    #[test]
    fn test_track_single_value() {
        let mut rms = LatencyRms::new(1);

        rms.track(10);

        assert_eq!(rms.count, 1);
        assert_eq!(rms.sum_squares, 100);
        assert_eq!(rms.rms(), 10); // sqrt(100/1) = 10
    }

    #[test]
    fn test_track_multiple_values() {
        let mut rms = LatencyRms::new(1);

        rms.track(3);
        rms.track(4);
        rms.track(5);

        // sum_squares = 9 + 16 + 25 = 50
        // count = 3
        // RMS = sqrt(50/3) ≈ sqrt(16.67) ≈ 4
        assert_eq!(rms.count, 3);
        assert_eq!(rms.sum_squares, 50);
        // RMS should be approximately 4 (integer sqrt of 16)
        assert!(rms.rms() >= 4 && rms.rms() <= 5);
    }

    #[test]
    fn test_update_interval() {
        let mut rms = LatencyRms::new(5);

        // Track 4 values - should not update RMS yet
        for i in 1..=4 {
            rms.track(i);
        }

        // RMS should still be 0 or not updated
        // The actual RMS value depends on when update was last called
        // Since update_interval is 5, after 5 tracks it should update

        // Track 5th value - should trigger update
        rms.track(5);

        // Now RMS should be calculated
        assert!(rms.rms() > 0);
        assert_eq!(rms.since_last_update, 0);
    }

    #[test]
    fn test_rms_calculation() {
        let mut rms = LatencyRms::new(1);

        // Values: 3, 4, 5
        // sum_squares = 9 + 16 + 25 = 50
        // mean_square = 50/3 ≈ 16.67
        // RMS = sqrt(16.67) ≈ 4.08, integer sqrt ≈ 4
        rms.track(3);
        rms.track(4);
        rms.track(5);

        let rms_value = rms.rms();
        // Should be approximately 4 (integer sqrt of 16.67)
        assert!(rms_value >= 4 && rms_value <= 5);
    }

    #[test]
    fn test_rms_with_zero() {
        let mut rms = LatencyRms::new(1);

        rms.track(0);

        assert_eq!(rms.rms(), 0);
        assert_eq!(rms.count, 1);
        assert_eq!(rms.sum_squares, 0);
    }

    #[test]
    fn test_rms_with_large_values() {
        let mut rms = LatencyRms::new(1);

        rms.track(1000);
        rms.track(2000);

        // sum_squares = 1,000,000 + 4,000,000 = 5,000,000
        // mean_square = 5,000,000 / 2 = 2,500,000
        // RMS = sqrt(2,500,000) ≈ 1581
        let rms_value = rms.rms();
        assert!(rms_value >= 1500 && rms_value <= 1600);
    }

    #[test]
    fn test_can_estimate() {
        let mut rms = LatencyRms::new(10);

        assert!(!rms.can_estimate());

        rms.track(10);

        assert!(rms.can_estimate());
    }

    #[test]
    fn test_trait_track() {
        let mut rms = LatencyRms::new(1);

        LatencyEstimator::track(&mut rms, 42);
        LatencyEstimator::track(&mut rms, 100);

        assert_eq!(rms.count, 2);
        assert_eq!(rms.sum_squares, 42 * 42 + 100 * 100);
    }

    #[test]
    fn test_trait_can_estimate() {
        let mut rms = LatencyRms::new(1);

        assert!(!LatencyEstimator::can_estimate(&rms));

        LatencyEstimator::track(&mut rms, 10);

        assert!(LatencyEstimator::can_estimate(&rms));
    }

    #[test]
    fn test_trait_estimate() {
        let mut rms = LatencyRms::new(1);

        LatencyEstimator::track(&mut rms, 10);
        LatencyEstimator::track(&mut rms, 20);

        // RMS ignores percentile parameter
        let estimate1 = LatencyEstimator::estimate(&rms);

        assert_eq!(estimate1, rms.rms());
    }

    #[test]
    fn test_periodic_update() {
        let mut rms = LatencyRms::new(3);

        // Track 2 values - no update yet
        rms.track(10);
        rms.track(20);
        assert_eq!(rms.since_last_update, 2);

        // Track 3rd value - should trigger update
        rms.track(30);
        assert_eq!(rms.since_last_update, 0);
        assert!(rms.rms() > 0);

        // Track more values
        rms.track(40);
        rms.track(50);
        assert_eq!(rms.since_last_update, 2);

        // Track 3rd value in this cycle - should trigger update again
        rms.track(60);
        assert_eq!(rms.since_last_update, 0);
    }

    #[test]
    fn test_integer_sqrt() {
        // Test integer_sqrt function directly
        assert_eq!(integer_sqrt(0), 0);
        assert_eq!(integer_sqrt(1), 1);
        assert_eq!(integer_sqrt(2), 1);
        assert_eq!(integer_sqrt(3), 1);
        assert_eq!(integer_sqrt(4), 2);
        assert_eq!(integer_sqrt(9), 3);
        assert_eq!(integer_sqrt(16), 4);
        assert_eq!(integer_sqrt(25), 5);
        assert_eq!(integer_sqrt(100), 10);
        assert_eq!(integer_sqrt(10000), 100);
    }

    #[test]
    fn test_integer_sqrt_large() {
        // Test with larger values
        assert_eq!(integer_sqrt(1000000), 1000);
        assert_eq!(integer_sqrt(2500000), 1581); // sqrt(2500000) ≈ 1581.14
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut rms = LatencyRms::new(100);

        for i in 1..=10 {
            rms.track(i);
        }

        let serialized = serde_json::to_string(&rms).unwrap();
        let deserialized: LatencyRms = serde_json::from_str(&serialized).unwrap();

        assert_eq!(rms.count, deserialized.count);
        assert_eq!(rms.sum_squares, deserialized.sum_squares);
        assert_eq!(rms.update_interval, deserialized.update_interval);
        assert_eq!(rms.rms, deserialized.rms);
    }

    #[test]
    fn test_rms_consistency() {
        let mut rms = LatencyRms::new(1);

        // Track same value multiple times
        for _ in 0..10 {
            rms.track(5);
        }

        // RMS of all 5s should be 5
        assert_eq!(rms.rms(), 5);
    }

    #[test]
    fn test_rms_with_identical_values() {
        let mut rms = LatencyRms::new(1);

        rms.track(10);
        rms.track(10);
        rms.track(10);

        // RMS of identical values equals the value itself
        assert_eq!(rms.rms(), 10);
    }
}

