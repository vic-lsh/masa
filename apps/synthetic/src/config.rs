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
pub struct ChildService {
    pub id: String,
    #[serde(default = "one_u8")]
    pub replicas: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHop {
    pub service_id: String,
    #[serde(default)]
    pub duration_us: Option<u64>,
    #[serde(default)]
    pub busy_spin_dur_us: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticConfig {
    #[serde(default = "default_random_latency")]
    pub child_random_latency: LatencyDistribution,
    // list of tuples of replica count and CPU share
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
}

fn one_u8() -> u8 {
    1
}

fn default_random_latency() -> LatencyDistribution {
    LatencyDistribution::Exponential {
        lambda: 1.0 / 10000.0,
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

#[cfg(test)]
mod tests {
    use super::{LatencyDistribution, SyntheticConfig};
    use serde_json::json;

    #[test]
    fn parses_request_hops_config() {
        let config = json!({
            "child_services": [
                { "id": "S1" },
                { "id": "C6", "replicas": 2 }
            ],
            "request_a_hops": [
                {
                    "service_id": "S1",
                    "duration_us": 12000,
                    "busy_spin_dur_us": 3000
                },
                {
                    "service_id": "C6",
                    "busy_spin_dur_us": 1000
                }
            ]
        });

        let parsed: SyntheticConfig = serde_json::from_value(config).expect("parse config");
        assert_eq!(parsed.child_services.len(), 2);
        assert_eq!(parsed.child_services[1].replicas, 2);
        assert_eq!(parsed.request_a_hops.len(), 2);
        assert_eq!(parsed.request_a_hops[0].duration_us, Some(12000));
        assert_eq!(parsed.request_a_hops[0].busy_spin_dur_us, Some(3000));
    }

    #[test]
    fn uses_default_random_latency_distribution() {
        let config = json!({});
        let parsed: SyntheticConfig = serde_json::from_value(config).expect("parse config");
        assert!(
            matches!(
                parsed.child_random_latency,
                LatencyDistribution::Exponential { .. }
            ),
            "expected default exponential for random latency"
        );
    }
}
