// Configuration data structures

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::constants::*;

/// Target for method calls
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallTarget {
    pub service_id: String,
    pub method_name: String,
}

/// Service method definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceMethod {
    pub name: String,
    pub latency_distribution: LatencyDistribution,
    #[serde(rename = "call_sequence")]
    pub call_sequence_raw: Vec<HashMap<String, f64>>,
    #[serde(skip)]
    pub parsed_call_sequence: Vec<Vec<(CallTarget, f64)>>,
    #[serde(default)]
    pub busy_spin_ratio: Option<f64>,
}

/// Service definition in call graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDefinition {
    pub id: String,
    #[serde(default = "one_u8")]
    pub replicas: u8,
    pub methods: Vec<ServiceMethod>,
}

/// Call graph configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraphConfig {
    pub entry_point: String,
    pub services: Vec<ServiceDefinition>,
    #[serde(default)]
    pub child_cpus_per_replica: f64,
}

/// Hop configuration for presampled requests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hop {
    pub service: usize,
    pub sleep: f64,
    pub latency_distribution: LatencyDistribution,
}

/// Child service configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildService {
    pub id: String,
    #[serde(default = "one_u8")]
    pub replicas: u8,
}

/// Request hop configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHop {
    pub service_id: String,
    #[serde(default)]
    pub duration_us: Option<u64>,
    #[serde(default)]
    pub busy_spin_dur_us: Option<u64>,
}

/// Main synthetic configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticConfig {
    #[serde(default = "default_random_latency")]
    pub child_random_latency: LatencyDistribution,

    // List of tuples of replica count and CPU share
    #[serde(default = "empty_vec")]
    pub child_presampled_services: Vec<Vec<f64>>,

    #[serde(default = "empty_map")]
    pub child_presampled_request_types: HashMap<String, Vec<Hop>>,

    #[serde(default)]
    pub child_services: Vec<ChildService>,

    #[serde(default)]
    pub request_a_hops: Vec<RequestHop>,

    #[serde(default)]
    pub request_b_hops: Vec<RequestHop>,

    #[serde(default = "onef64")]
    pub child_cpus_per_replica: f64,

    #[serde(default)]
    pub call_graph: Option<CallGraphConfig>,
}

// Re-export from distributions module
pub use super::distributions::{default_random_latency, LatencyDistribution};

// Helper functions for serde defaults
fn one_u8() -> u8 {
    DEFAULT_REPLICAS
}

fn empty_vec() -> Vec<Vec<f64>> {
    Vec::new()
}

fn empty_map() -> HashMap<String, Vec<Hop>> {
    HashMap::new()
}

fn onef64() -> f64 {
    DEFAULT_CHILD_CPU_PER_REPLICA
}
