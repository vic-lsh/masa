use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone, Debug, Serialize)]
pub enum LatencyDistribution {
    Normal {
        mean: f64,
        std: f64,
    },
    Discrete {
        values: Vec<u64>,
        weights: Vec<f64>,
    },
    Periodic {
        slow_latency: u64,
        fast_latency: u64,
        slow_duration_ms: u16,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hop {
    pub service: usize,
    #[serde(default = "zero")]
    pub replicas: usize,
    pub sleep: bool,
    pub latency_distribution: LatencyDistribution,
}

fn zero() -> usize {
    0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticConfig {
    pub child_constant_replicas: u8,
    pub child_constant_latency: u64,
    pub child_constant_latency_slowdown_duration: u16, // ms
    pub child_random_latency: LatencyDistribution,
    pub child_presampled_services: Vec<u8>,
    pub child_presampled_request_types: HashMap<String, Vec<Hop>>,
}
