use std::collections::{HashMap, HashSet};

use crate::distribution::LatencyDistribution;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallTarget {
    pub service_id: String,
    pub method_name: String,
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
pub struct CallGraphSpec {
    pub entry_point: String,
    #[serde(default)]
    pub services: Vec<ServiceDefinition>,
    #[serde(default)]
    pub service_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiSpec {
    pub name: String,
    pub call_graph: String,
    pub traffic_weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticConfig {
    pub services: Vec<ServiceDefinition>,
    pub call_graphs: HashMap<String, CallGraphSpec>,
    pub apis: Vec<ApiSpec>,
    #[serde(default = "onef64")]
    pub child_cpus_per_replica: f64,
}

fn one_u8() -> u8 {
    1
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
pub struct ResolvedCallGraphs {
    pub services: Vec<ServiceDefinition>,
    pub api_entry_points: HashMap<String, CallTarget>,
}

fn build_target_set(services: &[ServiceDefinition]) -> HashSet<String> {
    let mut targets = HashSet::new();
    for service in services {
        for method in &service.methods {
            targets.insert(format!("{}::{}", service.id, method.name));
        }
    }
    targets
}

fn validate_call_sequences(
    services: &[ServiceDefinition],
    valid_targets: &HashSet<String>,
) -> Result<(), String> {
    for service in services {
        for method in &service.methods {
            for step in &method.call_sequence_raw {
                for (target_str, _prob) in step {
                    if !valid_targets.contains(target_str) {
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

fn parse_call_sequences_for_services(
    services: &mut [ServiceDefinition],
    valid_targets: &HashSet<String>,
) -> Result<(), String> {
    validate_call_sequences(services, valid_targets)?;
    for service in services {
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

fn build_merged_services(config: &SyntheticConfig) -> Result<Vec<ServiceDefinition>, String> {
    let mut services = Vec::new();
    let mut seen = HashSet::new();

    fn add_service(
        services: &mut Vec<ServiceDefinition>,
        seen: &mut HashSet<String>,
        svc: &ServiceDefinition,
    ) -> Result<(), String> {
        if seen.contains(&svc.id) {
            return Err(format!("Duplicate service id '{}'", svc.id));
        }
        seen.insert(svc.id.clone());
        services.push(svc.clone());
        Ok(())
    }

    for svc in &config.services {
        add_service(&mut services, &mut seen, svc)?;
    }

    for graph in config.call_graphs.values() {
        for svc in &graph.services {
            add_service(&mut services, &mut seen, svc)?;
        }
        for svc_id in &graph.service_refs {
            if !seen.contains(svc_id) {
                return Err(format!(
                    "Call graph references missing service id '{}'",
                    svc_id
                ));
            }
        }
    }

    Ok(services)
}

pub fn resolve_call_graphs(config: &SyntheticConfig) -> Result<ResolvedCallGraphs, String> {
    if config.apis.is_empty() {
        return Err("apis must not be empty".to_string());
    }

    let mut merged_services = build_merged_services(config)?;
    let valid_targets = build_target_set(&merged_services);

    let mut api_entry_points = HashMap::new();
    let mut api_names = HashSet::new();
    let mut has_weight = false;

    for api in &config.apis {
        if api.traffic_weight < 0.0 {
            return Err(format!(
                "API '{}' has negative traffic_weight",
                api.name
            ));
        }
        if api.traffic_weight > 0.0 {
            has_weight = true;
        }
        if !api_names.insert(api.name.clone()) {
            return Err(format!("Duplicate API name '{}'", api.name));
        }

        let graph = config
            .call_graphs
            .get(&api.call_graph)
            .ok_or_else(|| format!("API '{}' references unknown call graph", api.name))?;

        let entry_target = parse_service_method(&graph.entry_point)?;
        let entry_key = format!("{}::{}", entry_target.service_id, entry_target.method_name);
        if !valid_targets.contains(&entry_key) {
            return Err(format!(
                "Entry point '{}' does not exist in merged services",
                graph.entry_point
            ));
        }

        api_entry_points.insert(api.name.clone(), entry_target);
    }

    if !has_weight {
        return Err("At least one api must have traffic_weight > 0".to_string());
    }

    validate_call_sequences(&merged_services, &valid_targets)?;
    parse_call_sequences_for_services(&mut merged_services, &valid_targets)?;

    Ok(ResolvedCallGraphs {
        services: merged_services,
        api_entry_points,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_service_method, resolve_call_graphs, SyntheticConfig};
    use crate::distribution::LatencyDistribution;
    use serde_json::json;

    #[test]
    fn parses_exponential_with_mean() {
        let config = json!({
            "services": [
                {
                    "id": "MS_1",
                    "replicas": 1,
                    "methods": [
                        {
                            "name": "method1",
                            "latency_distribution": {"Exponential": {"mean": 10000.0}},
                            "call_sequence": []
                        }
                    ]
                }
            ],
            "call_graphs": {
                "graph1": {
                    "entry_point": "MS_1::method1",
                    "services": [],
                    "service_refs": ["MS_1"]
                }
            },
            "apis": [
                {
                    "name": "api1",
                    "call_graph": "graph1",
                    "traffic_weight": 1.0
                }
            ]
        });

        let parsed: SyntheticConfig = serde_json::from_value(config).expect("parse config");
        let resolved = resolve_call_graphs(&parsed).expect("resolve call graphs");
        let method = &resolved.services[0].methods[0];
        match &method.latency_distribution {
            LatencyDistribution::Exponential { lambda, mean, .. } => {
                // mean = 10000, so lambda should be 1/10000 = 0.0001
                assert!((lambda - 0.0001).abs() < 1e-10);
                assert_eq!(mean, &Some(10000.0));
            }
            _ => panic!("Expected Exponential distribution"),
        }
    }

    #[test]
    fn parses_exponential_with_both_mean_and_lambda_prefers_mean() {
        let config = json!({
            "services": [
                {
                    "id": "MS_1",
                    "replicas": 1,
                    "methods": [
                        {
                            "name": "method1",
                            "latency_distribution": {"Exponential": {"lambda": 0.0002, "mean": 10000.0}},
                            "call_sequence": []
                        }
                    ]
                }
            ],
            "call_graphs": {
                "graph1": {
                    "entry_point": "MS_1::method1",
                    "services": [],
                    "service_refs": ["MS_1"]
                }
            },
            "apis": [
                {
                    "name": "api1",
                    "call_graph": "graph1",
                    "traffic_weight": 1.0
                }
            ]
        });

        let parsed: SyntheticConfig = serde_json::from_value(config).expect("parse config");
        let resolved = resolve_call_graphs(&parsed).expect("resolve call graphs");
        let method = &resolved.services[0].methods[0];
        match &method.latency_distribution {
            LatencyDistribution::Exponential { lambda, mean, .. } => {
                // Should prefer mean, so lambda should be 1/10000 = 0.0001, not 0.0002
                assert!((lambda - 0.0001).abs() < 1e-10);
                assert_eq!(mean, &Some(10000.0));
            }
            _ => panic!("Expected Exponential distribution"),
        }
    }

    #[test]
    fn parses_call_graph_config() {
        let config = json!({
            "services": [
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
            ],
            "call_graphs": {
                "graph1": {
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
                        }
                    ],
                    "service_refs": ["MS_37691"]
                }
            },
            "apis": [
                {
                    "name": "api1",
                    "call_graph": "graph1",
                    "traffic_weight": 1.0
                }
            ]
        });

        let parsed: SyntheticConfig = serde_json::from_value(config).expect("parse config");
        let resolved = resolve_call_graphs(&parsed).expect("resolve call graphs");
        assert_eq!(resolved.services.len(), 2);
        let ms_56394 = resolved
            .services
            .iter()
            .find(|svc| svc.id == "MS_56394")
            .expect("missing MS_56394");
        assert_eq!(ms_56394.methods.len(), 2);
        assert_eq!(ms_56394.methods[0].name, "GqI6UW1mU4");
        assert_eq!(ms_56394.methods[0].call_sequence_raw.len(), 0);
        assert_eq!(ms_56394.methods[1].call_sequence_raw.len(), 2);
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
    fn resolves_call_graphs_and_parses_call_sequences() {
        let config = json!({
            "services": [
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
                },
                {
                    "id": "MS_73106",
                    "replicas": 1,
                    "methods": [
                        {
                            "name": "method3",
                            "latency_distribution": {"Exponential": {"lambda": 0.0001}},
                            "call_sequence": []
                        }
                    ]
                }
            ],
            "call_graphs": {
                "graph1": {
                    "entry_point": "MS_56394::GqI6UW1mU4",
                    "services": [
                        {
                            "id": "MS_56394",
                            "replicas": 1,
                            "methods": [
                                {
                                    "name": "GqI6UW1mU4",
                                    "latency_distribution": {"Exponential": {"lambda": 0.0001}},
                                    "call_sequence": [
                                        {
                                            "MS_37691::y_DKOh-Gts": 1.0,
                                            "MS_37691::ykccIz2fkK": 0.8
                                        },
                                        {
                                            "MS_73106::method3": 1.0
                                        }
                                    ]
                                }
                            ]
                        }
                    ],
                    "service_refs": ["MS_37691", "MS_73106"]
                }
            },
            "apis": [
                {
                    "name": "api1",
                    "call_graph": "graph1",
                    "traffic_weight": 1.0
                }
            ]
        });

        let parsed: SyntheticConfig = serde_json::from_value(config).expect("parse config");
        let resolved = resolve_call_graphs(&parsed).expect("resolve call graphs");
        let method = resolved
            .services
            .iter()
            .find(|svc| svc.id == "MS_56394")
            .expect("missing MS_56394")
            .methods
            .iter()
            .find(|m| m.name == "GqI6UW1mU4")
            .expect("missing method");

        assert_eq!(method.parsed_call_sequence.len(), 2);
        assert_eq!(method.parsed_call_sequence[0].len(), 2);
        assert_eq!(method.parsed_call_sequence[1].len(), 1);
    }
}
