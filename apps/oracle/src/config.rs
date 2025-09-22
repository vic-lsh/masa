use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone, Debug, Serialize)]
pub enum LatencyDistribution {
    Normal {
        mean: f64,
        std: f64,
    },
    Exponential {
        lambda: f64,
    },
    Discrete {
        weights: Vec<f64>,
        values: Vec<u64>,
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
    pub sleep: f64,
    pub latency_distribution: LatencyDistribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticConfig {
    #[serde(default = "one_u8")]
    pub child_constant_replicas: u8,
    #[serde(default = "default_constant_latency")]
    pub child_constant_latency: u64,
    #[serde(default = "zero_u16")]
    pub child_constant_latency_slowdown_duration: u16, // ms
    #[serde(default = "default_random_latency")]
    pub child_random_latency: LatencyDistribution,
    // list of tuples of replica count and CPU share
    #[serde(default = "empty_vec")]
    pub child_presampled_services: Vec<Vec<f64>>,
    #[serde(default = "empty_map")]
    pub child_presampled_request_types: HashMap<String, Vec<Hop>>,
    #[serde(default = "onef64")]
    pub child_cpus_per_replica: f64,
}

fn one_u8() -> u8 {
    1
}

fn default_constant_latency() -> u64 {
    500
}

fn zero_u16() -> u16 {
    0
}

fn default_random_latency() -> LatencyDistribution {
    LatencyDistribution::Discrete {
        weights: vec![0.95, 0.05],
        values: vec![5000, 30000],
    }
}

fn empty_vec() -> Vec<Vec<f64>> {
    Vec::new()
}

fn empty_map() -> HashMap<String, Vec<Hop>> {
    HashMap::new()
}

fn onef64() -> f64 {
    1.0
}
