use serde::{Deserialize, Serialize};
use super::LatencyEstimator;

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

    fn estimate(&self, _percentile: usize) -> u64 {
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
