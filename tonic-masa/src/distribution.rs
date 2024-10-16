use serde::{Deserialize, Serialize};

pub use crate::{Latency, RequestId};

/// Represent a distribution.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Distribution {
    mean: Latency,
    percentile_latencies: Vec<Latency>,
}

impl Distribution {
    /// Create a new distribution.
    pub fn new(mean: Latency, percentile_latencies: Vec<Latency>) -> Self {
        Self {
            mean,
            percentile_latencies,
        }
    }

    /// Sample a latency.
    pub fn sample(&self, _request_id: RequestId) -> Latency {
        // let index = request_id as usize % self.percentile_latencies.len();
        // self.percentile_latencies[index]
        self.mean
    }

    /// Get the mean.
    pub fn mean(&self) -> Latency {
        self.mean
    }
}
