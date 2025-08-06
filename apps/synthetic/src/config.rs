use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone, Debug, Serialize)]
pub enum LatencyDistribution {
    Normal {
        mean: f64,
        std: f64,
    },
    Discrete {
        values: Vec<f64>,
        weights: Vec<f64>,
    },
    Periodic {
        slow_latency: f64,
        fast_latency: f64,
        slow_duration_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hop {
    pub server: usize,
    pub sleep: bool,
    pub latency_distribution: LatencyDistribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticConfig {
    pub child_constant_replicas: u8,
    pub child_constant_latency: u64,
    pub child_constant_latency_slowdown_duration: u16, // ms
    pub child_random_latency: LatencyDistribution,
    pub child_presampled_replicas: u8,
    pub child_presampled_request_types: HashMap<String, Vec<Hop>>,
}
