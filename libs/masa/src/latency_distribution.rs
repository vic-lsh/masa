use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
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

        log::warn!(
            "update distribution {}: mean: {} us, p50: {} us, p90: {} us, p95: {} us, p99: {} us",
            self.name,
            self.mean,
            self.percentile(50),
            self.percentile(90),
            self.percentile(95),
            self.percentile(99)
        );
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

pub trait Estimator {
    type EstimateInput;

    fn estimate(&self, input: Self::EstimateInput) -> Option<u64>;
    fn update(&mut self, observed: u64);
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PercentileEstimator {
    distribution: LatencyDistribution,
}

impl Estimator for PercentileEstimator {
    type EstimateInput = usize; // percentile

    fn estimate(&self, percentile: Self::EstimateInput) -> Option<u64> {
        if !self.distribution.can_estimate() {
            return None;
        }
        Some(self.distribution.percentile(percentile))
    }

    fn update(&mut self, observed: u64) {
        self.distribution.track(observed);
    }
}
