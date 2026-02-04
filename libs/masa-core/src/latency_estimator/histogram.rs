use super::LatencyEstimator;
use serde::{Deserialize, Serialize};

// TODO: tweak this value. should it be configurable?
const PERCENTILE: usize = 50;

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

    pub fn estimate(&self) -> u64 {
        if self.percentiles.len() == 0 {
            self.mean
        } else {
            self.percentile(PERCENTILE)
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

    fn estimate(&self) -> u64 {
        // Forward to the struct's estimate method
        if self.percentiles.len() == 0 {
            self.mean
        } else {
            self.percentile(PERCENTILE)
        }
    }
}

impl Default for LatencyDistribution {
    fn default() -> Self {
        // Use a reasonable default capacity
        LatencyDistribution::new(String::new(), 512)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let dist = LatencyDistribution::new("test".to_string(), 200);
        assert_eq!(dist.capacity, 200);
        assert_eq!(dist.cur_queue.len(), 0);
        assert_eq!(dist.prev_queue.len(), 0);
        assert_eq!(dist.mean, 0);
        assert_eq!(dist.percentiles.len(), 0);
    }

    #[test]
    #[should_panic(expected = "Capacity should be no less than 100")]
    fn test_new_invalid_capacity() {
        LatencyDistribution::new("test".to_string(), 50);
    }

    #[test]
    fn test_default() {
        let dist = LatencyDistribution::default();
        assert_eq!(dist.capacity, 512);
        assert_eq!(dist.cur_queue.len(), 0);
    }

    #[test]
    fn test_track_before_capacity() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        // Track values but don't reach capacity
        for i in 0..100 {
            dist.track(i);
        }

        assert_eq!(dist.cur_queue.len(), 100);
        assert!(!dist.can_estimate());
    }

    #[test]
    fn test_track_reaches_capacity() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        // Track exactly capacity values
        for i in 0..200 {
            dist.track(i);
        }

        // After reaching capacity, update should be called
        assert_eq!(dist.cur_queue.len(), 0);
        assert!(dist.can_estimate());
        assert_eq!(dist.percentiles.len(), 100);
    }

    #[test]
    fn test_percentiles_calculation() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        // Track values 0-199
        for i in 0..200 {
            dist.track(i);
        }

        assert!(dist.can_estimate());

        // 0th percentile should be minimum (0)
        assert_eq!(dist.percentile(0), 0);

        // 50th percentile should be around 100
        assert_eq!(dist.percentile(50), 100);

        // 99th percentile: idx = (200 * 99) / 100 = 198, so value is 198
        assert_eq!(dist.percentile(99), 198);
    }

    #[test]
    fn test_mean_calculation() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        // Track values 0-199
        for i in 0..200 {
            dist.track(i);
        }

        // Mean of 0-199 is (0+199)*200/2 / 200 = 199/2 = 99.5, rounded to 99
        // Actually, sum is 19900, divided by 200 is 99
        assert_eq!(dist.mean, 99);
    }

    #[test]
    fn test_percentiles_accessor() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        for i in 0..200 {
            dist.track(i);
        }

        let percentiles = dist.percentiles();
        assert_eq!(percentiles.len(), 100);
        assert_eq!(percentiles[0], 0);
        assert_eq!(percentiles[50], 100);
        // 99th percentile: idx = (200 * 99) / 100 = 198
        assert_eq!(percentiles[99], 198);
    }

    #[test]
    #[should_panic(expected = "Percentile should be less than 100")]
    fn test_percentile_out_of_bounds() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        for i in 0..200 {
            dist.track(i);
        }

        dist.percentile(100);
    }

    #[test]
    fn test_estimate_without_data() {
        let dist = LatencyDistribution::new("test".to_string(), 200);

        // Should return mean (0) when no percentiles available
        assert_eq!(dist.estimate(), 0);
    }

    #[test]
    fn test_estimate_with_data() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        for i in 0..200 {
            dist.track(i);
        }

        // Should return percentile value
        assert_eq!(dist.estimate(), 100);
    }

    #[test]
    fn test_trait_track() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        // Use trait method
        LatencyEstimator::track(&mut dist, 42);
        LatencyEstimator::track(&mut dist, 100);

        assert_eq!(dist.cur_queue.len(), 2);
    }

    #[test]
    fn test_trait_can_estimate() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        assert!(!LatencyEstimator::can_estimate(&dist));

        for i in 0..200 {
            LatencyEstimator::track(&mut dist, i);
        }

        assert!(LatencyEstimator::can_estimate(&dist));
    }

    #[test]
    fn test_trait_estimate() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        // Without data, should return mean
        assert_eq!(LatencyEstimator::estimate(&dist), 0);

        // With data, should return percentile
        for i in 0..200 {
            LatencyEstimator::track(&mut dist, i);
        }

        assert_eq!(LatencyEstimator::estimate(&dist), 100);
    }

    #[test]
    fn test_multiple_capacity_cycles() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        // First cycle: 0-199
        for i in 0..200 {
            dist.track(i);
        }

        // After first cycle: prev_queue has 0-199, cur_queue is empty
        // Percentiles are calculated from 0-199

        // Second cycle: 200-399
        for i in 200..400 {
            dist.track(i);
        }

        // After second cycle, update() combines prev_queue (0-199) and cur_queue (200-399)
        // So percentiles should be from the combined set 0-399
        assert!(dist.can_estimate());
        assert_eq!(dist.percentile(0), 0);
        // 50th percentile of 0-399: idx = (400 * 50) / 100 = 200
        assert_eq!(dist.percentile(50), 200);
        // 99th percentile: idx = (400 * 99) / 100 = 396
        assert_eq!(dist.percentile(99), 396);
    }

    #[test]
    fn test_uneven_distribution() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        // Track many small values and a few large ones
        for _ in 0..190 {
            dist.track(10);
        }
        for _ in 0..10 {
            dist.track(1000);
        }

        // Most percentiles should be 10
        assert_eq!(dist.percentile(0), 10);
        assert_eq!(dist.percentile(90), 10);
        // Higher percentiles should be 1000
        assert_eq!(dist.percentile(95), 1000);
        assert_eq!(dist.percentile(99), 1000);
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut dist = LatencyDistribution::new("test".to_string(), 200);

        for i in 0..200 {
            dist.track(i);
        }

        let serialized = serde_json::to_string(&dist).unwrap();
        let deserialized: LatencyDistribution = serde_json::from_str(&serialized).unwrap();

        assert_eq!(dist.percentiles.len(), deserialized.percentiles.len());
        assert_eq!(dist.mean, deserialized.mean);
        assert_eq!(dist.capacity, deserialized.capacity);
    }
}
