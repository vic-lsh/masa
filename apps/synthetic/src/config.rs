use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallTarget {
    pub service_id: String,
    pub method_name: String,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDefinition {
    pub id: String,
    #[serde(default = "one_u8")]
    pub replicas: u8,
    pub methods: Vec<ServiceMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraphConfig {
    pub entry_point: String,
    pub services: Vec<ServiceDefinition>,
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
    #[serde(default)]
    pub call_graph: Option<CallGraphConfig>,
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

/// Parse a "service_name::method_name" string into a CallTarget
pub fn parse_service_method(s: &str) -> Result<CallTarget, String> {
    let parts: Vec<&str> = s.split("::").collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid service::method format: '{}'. Expected 'service_name::method_name'",
            s
        ));
    }
    Ok(CallTarget {
        service_id: parts[0].to_string(),
        method_name: parts[1].to_string(),
    })
}

/// Validate that all referenced services and methods exist in the call graph
pub fn validate_call_graph(config: &CallGraphConfig) -> Result<(), String> {
    // Build a set of all valid service::method combinations
    let mut valid_targets = HashMap::new();
    for service in &config.services {
        for method in &service.methods {
            let target = format!("{}::{}", service.id, method.name);
            valid_targets.insert(target, (service.id.clone(), method.name.clone()));
        }
    }

    // Validate entry_point
    if !valid_targets.contains_key(&config.entry_point) {
        return Err(format!(
            "Entry point '{}' does not exist in call graph",
            config.entry_point
        ));
    }

    // Validate all call sequences
    for service in &config.services {
        for method in &service.methods {
            for step in &method.call_sequence_raw {
                for (target_str, _prob) in step {
                    if !valid_targets.contains_key(target_str) {
                        return Err(format!(
                            "Service '{}' method '{}' references non-existent target '{}'",
                            service.id, method.name, target_str
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

/// Parse and validate call sequences for all methods in a call graph config
pub fn parse_call_sequences(config: &mut CallGraphConfig) -> Result<(), String> {
    // First validate the graph structure
    validate_call_graph(config)?;

    // Parse all call sequences
    for service in &mut config.services {
        for method in &mut service.methods {
            let mut parsed = Vec::new();
            for step in &method.call_sequence_raw {
                let mut parsed_step = Vec::new();
                for (target_str, prob) in step {
                    let target = parse_service_method(target_str)?;
                    parsed_step.push((target, *prob));
                }
                parsed.push(parsed_step);
            }
            method.parsed_call_sequence = parsed;
        }
    }

    Ok(())
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

    #[test]
    fn parses_call_graph_config() {
        let config = json!({
            "call_graph": {
                "entry_point": "MS_56394::GqI6UW1mU4",
                "services": [
                    {
                        "id": "MS_56394",
                        "replicas": 1,
                        "methods": [
                            {
                                "name": "GqI6UW1mU4",
                                "latency_distribution": {"Exponential": {"lambda": 0.0001}},
                                "call_sequence": []
                            },
                            {
                                "name": "method1",
                                "latency_distribution": {"Normal": {"mean": 10000.0, "std": 2000.0}},
                                "call_sequence": [
                                    {"MS_37691::y_DKOh-Gts": 1.0},
                                    {"MS_37691::ykccIz2fkK": 1.0}
                                ]
                            }
                        ]
                    },
                    {
                        "id": "MS_37691",
                        "replicas": 1,
                        "methods": [
                            {
                                "name": "y_DKOh-Gts",
                                "latency_distribution": {"Exponential": {"lambda": 0.0001}},
                                "call_sequence": []
                            },
                            {
                                "name": "ykccIz2fkK",
                                "latency_distribution": {"Exponential": {"lambda": 0.0001}},
                                "call_sequence": []
                            }
                        ]
                    }
                ]
            }
        });

        let parsed: SyntheticConfig = serde_json::from_value(config).expect("parse config");
        assert!(parsed.call_graph.is_some());
        let call_graph = parsed.call_graph.as_ref().unwrap();
        assert_eq!(call_graph.entry_point, "MS_56394::GqI6UW1mU4");
        assert_eq!(call_graph.services.len(), 2);
        assert_eq!(call_graph.services[0].id, "MS_56394");
        assert_eq!(call_graph.services[0].methods.len(), 2);
        assert_eq!(call_graph.services[0].methods[0].name, "GqI6UW1mU4");
        assert_eq!(call_graph.services[0].methods[0].call_sequence_raw.len(), 0);
        assert_eq!(call_graph.services[0].methods[1].call_sequence_raw.len(), 2);
    }

    #[test]
    fn test_parse_service_method() {
        let target = parse_service_method("MS_56394::GqI6UW1mU4").unwrap();
        assert_eq!(target.service_id, "MS_56394");
        assert_eq!(target.method_name, "GqI6UW1mU4");

        assert!(parse_service_method("invalid").is_err());
        assert!(parse_service_method("service::method::extra").is_err());
    }

    #[test]
    fn test_validate_call_graph() {
        let mut config = CallGraphConfig {
            entry_point: "MS_56394::GqI6UW1mU4".to_string(),
            services: vec![
                ServiceDefinition {
                    id: "MS_56394".to_string(),
                    replicas: 1,
                    methods: vec![
                        ServiceMethod {
                            name: "GqI6UW1mU4".to_string(),
                            latency_distribution: LatencyDistribution::Exponential { lambda: 0.0001 },
                            call_sequence_raw: vec![
                                [("MS_37691::y_DKOh-Gts".to_string(), 1.0)].iter().cloned().collect(),
                            ],
                            parsed_call_sequence: vec![],
                            busy_spin_ratio: None,
                        },
                    ],
                },
                ServiceDefinition {
                    id: "MS_37691".to_string(),
                    replicas: 1,
                    methods: vec![
                        ServiceMethod {
                            name: "y_DKOh-Gts".to_string(),
                            latency_distribution: LatencyDistribution::Exponential { lambda: 0.0001 },
                            call_sequence_raw: vec![],
                            parsed_call_sequence: vec![],
                            busy_spin_ratio: None,
                        },
                    ],
                },
            ],
        };

        // Should validate successfully
        assert!(validate_call_graph(&config).is_ok());

        // Test invalid entry point
        config.entry_point = "INVALID::method".to_string();
        assert!(validate_call_graph(&config).is_err());

        // Test invalid target reference
        config.entry_point = "MS_56394::GqI6UW1mU4".to_string();
        config.services[0].methods[0].call_sequence_raw[0]
            .insert("INVALID::method".to_string(), 1.0);
        assert!(validate_call_graph(&config).is_err());
    }

    #[test]
    fn test_parse_call_sequences() {
        let mut config = CallGraphConfig {
            entry_point: "MS_56394::GqI6UW1mU4".to_string(),
            services: vec![
                ServiceDefinition {
                    id: "MS_56394".to_string(),
                    replicas: 1,
                    methods: vec![
                        ServiceMethod {
                            name: "GqI6UW1mU4".to_string(),
                            latency_distribution: LatencyDistribution::Exponential { lambda: 0.0001 },
                            call_sequence_raw: vec![
                                [
                                    ("MS_37691::y_DKOh-Gts".to_string(), 1.0),
                                    ("MS_37691::ykccIz2fkK".to_string(), 0.8),
                                ]
                                .iter()
                                .cloned()
                                .collect(),
                                [("MS_73106::method3".to_string(), 1.0)]
                                    .iter()
                                    .cloned()
                                    .collect(),
                            ],
                            parsed_call_sequence: vec![],
                            busy_spin_ratio: None,
                        },
                    ],
                },
                ServiceDefinition {
                    id: "MS_37691".to_string(),
                    replicas: 1,
                    methods: vec![
                        ServiceMethod {
                            name: "y_DKOh-Gts".to_string(),
                            latency_distribution: LatencyDistribution::Exponential { lambda: 0.0001 },
                            call_sequence_raw: vec![],
                            parsed_call_sequence: vec![],
                            busy_spin_ratio: None,
                        },
                        ServiceMethod {
                            name: "ykccIz2fkK".to_string(),
                            latency_distribution: LatencyDistribution::Exponential { lambda: 0.0001 },
                            call_sequence_raw: vec![],
                            parsed_call_sequence: vec![],
                            busy_spin_ratio: None,
                        },
                    ],
                },
                ServiceDefinition {
                    id: "MS_73106".to_string(),
                    replicas: 1,
                    methods: vec![ServiceMethod {
                        name: "method3".to_string(),
                        latency_distribution: LatencyDistribution::Exponential { lambda: 0.0001 },
                        call_sequence_raw: vec![],
                        parsed_call_sequence: vec![],
                        busy_spin_ratio: None,
                    }],
                },
            ],
        };

        assert!(parse_call_sequences(&mut config).is_ok());
        let method = &config.services[0].methods[0];
        assert_eq!(method.parsed_call_sequence.len(), 2);
        assert_eq!(method.parsed_call_sequence[0].len(), 2);
        assert_eq!(method.parsed_call_sequence[1].len(), 1);
        assert_eq!(method.parsed_call_sequence[0][0].0.service_id, "MS_37691");
        assert_eq!(method.parsed_call_sequence[0][0].0.method_name, "y_DKOh-Gts");
        assert_eq!(method.parsed_call_sequence[0][0].1, 1.0);
    }
}
