use serde::{Deserialize, Serialize};

/// Trait for latency estimators that can track latency values and provide estimates.
/// This allows different estimation strategies (e.g., histogram-based, RMS-based) to be used interchangeably.
pub trait LatencyEstimator: Send + Sync {
    /// Track a new latency value.
    fn track(&mut self, value: u64);

    /// Check if the estimator has enough data to provide an estimate.
    fn can_estimate(&self) -> bool;

    /// Get an estimate for the given percentile.
    /// The percentile parameter is used by some estimators (e.g., histogram-based) to select
    /// a specific percentile value. Other estimators may ignore this parameter.
    fn estimate(&self, percentile: usize) -> u64;
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LatencyDistribution {
    name: String,
    capacity: usize,
    cur_queue: Vec<u64>,
    prev_queue: Vec<u64>,
    mean: u64,
    percentiles: Vec<u64>,
}

impl LatencyDistribution {
    pub fn new(name: String, capacity: usize) -> Self {
        assert!(capacity >= 100, "Capacity should be no less than 100");
        LatencyDistribution {
            name,
            capacity,
            cur_queue: Vec::with_capacity(capacity),
            prev_queue: Vec::with_capacity(capacity),
            mean: 0,
            percentiles: Vec::new(),
        }
    }

    pub fn track(&mut self, value: u64) {
        self.cur_queue.push(value);
        if self.cur_queue.len() >= self.capacity {
            self.update();
            std::mem::swap(&mut self.cur_queue, &mut self.prev_queue);
            self.cur_queue.clear();
        }
    }

    fn update(&mut self) {
        let mut values = Vec::new();
        values.extend(self.prev_queue.iter());
        values.extend(self.cur_queue.iter());
        values.sort();

        let sum: u64 = values.iter().sum();
        self.mean = sum / values.len() as u64;

        self.percentiles.clear();
        for i in 0..100 {
            let idx = (values.len() * i) / 100;
            self.percentiles.push(values[idx]);
        }
    }

    pub fn percentiles(&self) -> &Vec<u64> {
        &self.percentiles
    }

    pub fn percentile(&self, p: usize) -> u64 {
        assert!(p < 100, "Percentile should be less than 100");
        self.percentiles[p]
    }

    pub fn can_estimate(&self) -> bool {
        self.percentiles.len() > 0
    }

    pub fn estimate(&self, percentile: usize) -> u64 {
        if self.percentiles.len() == 0 {
            self.mean
        } else {
            self.percentile(percentile)
        }
    }
}

impl LatencyEstimator for LatencyDistribution {
    fn track(&mut self, value: u64) {
        // Forward to the struct's track method
        self.cur_queue.push(value);
        if self.cur_queue.len() >= self.capacity {
            self.update();
            std::mem::swap(&mut self.cur_queue, &mut self.prev_queue);
            self.cur_queue.clear();
        }
    }

    fn can_estimate(&self) -> bool {
        // Forward to the struct's can_estimate method
        self.percentiles.len() > 0
    }

    fn estimate(&self, percentile: usize) -> u64 {
        // Forward to the struct's estimate method
        if self.percentiles.len() == 0 {
            self.mean
        } else {
            self.percentile(percentile)
        }
    }
}

impl Default for LatencyDistribution {
    fn default() -> Self {
        // Use a reasonable default capacity
        LatencyDistribution::new(String::new(), 512)
    }
}
