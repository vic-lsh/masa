use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LatencyTracker {
    capacity: usize,
    cur_queue: Vec<u64>,
    prev_queue: Vec<u64>,
    mean: u64,
    percentiles: Vec<u64>,
}

impl LatencyTracker {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity >= 100, "Capacity should be no less than 100");
        LatencyTracker {
            capacity,
            cur_queue: Vec::with_capacity(capacity),
            prev_queue: Vec::with_capacity(capacity),
            mean: 0,
            percentiles: Vec::new(),
        }
    }

    pub fn add(&mut self, value: u64) {
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

    pub fn estimate(&self) -> u64 {
        self.mean
    }
}
