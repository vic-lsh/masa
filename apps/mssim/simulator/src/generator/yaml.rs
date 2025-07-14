use anyhow::Result;
use serde_yaml;
use std::collections::HashMap;

use crate::parser::SimulatorConfig;

#[derive(Debug, serde::Serialize)]
struct SimulatorYaml {
    services: HashMap<String, ServiceYaml>,
    load: Option<LoadYaml>,
}

#[derive(Debug, serde::Serialize)]
struct ServiceYaml {
    container_port: u16,
    methods: HashMap<String, MethodYaml>,
}

#[derive(Debug, serde::Serialize)]
struct MethodYaml {
    calls: Vec<Vec<String>>,
    latency_distribution: Distribution,
    error_rate: Option<Distribution>,
}

#[derive(Debug, serde::Serialize)]
struct Distribution {
    distribution_type: String,
    parameters: HashMap<String, f64>,
}

#[derive(Debug, serde::Serialize)]
struct LoadYaml {
    entry_points: Vec<EntryPoint>,
}

#[derive(Debug, serde::Serialize)]
struct EntryPoint {
    service: String,
    method: String,
    requests_per_second: u32,
}

pub fn generate_simulator_yaml(config: &SimulatorConfig) -> Result<String> {
    // Transform the config into the expected YAML structure
    let simulator_yaml = SimulatorYaml {
        services: config
            .services
            .iter()
            .map(|(name, service)| {
                (
                    name.clone(),
                    ServiceYaml {
                        container_port: service.port.clone(),
                        methods: service
                            .methods
                            .iter()
                            .map(|(method_name, method)| {
                                (
                                    method_name.clone(),
                                    MethodYaml {
                                        calls: method.calls.clone(),
                                        latency_distribution: Distribution {
                                            distribution_type: method
                                                .latency_distribution
                                                .distribution_type
                                                .clone(),
                                            parameters: method
                                                .latency_distribution
                                                .parameters
                                                .clone(),
                                        },
                                        error_rate: method.error_rate.as_ref().map(|er| {
                                            Distribution {
                                                distribution_type: er.distribution_type.clone(),
                                                parameters: er.parameters.clone(),
                                            }
                                        }),
                                    },
                                )
                            })
                            .collect(),
                    },
                )
            })
            .collect(),
        load: config.load.as_ref().map(|load| LoadYaml {
            entry_points: load
                .entry_points
                .iter()
                .map(|ep| EntryPoint {
                    service: ep.service.clone(),
                    method: ep.method.clone(),
                    requests_per_second: ep.requests_per_second,
                })
                .collect(),
        }),
    };

    // Serialize to YAML
    let yaml = serde_yaml::to_string(&simulator_yaml)?;
    Ok(yaml)
}
