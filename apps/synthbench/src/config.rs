use std::collections::HashMap;

use crate::distribution::LatencyDistribution;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallTarget {
    pub service_id: String,
    pub method_name: String,
}

#[derive(Debug, Clone)]
pub struct ParsedCall {
    pub target: CallTarget,
    pub probability: f64,
}

impl ParsedCall {
    fn new(target: CallTarget, probability: f64) -> Self {
        Self {
            target,
            probability,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceMethod {
    pub name: String,
    pub latency_distribution: LatencyDistribution,
    #[serde(rename = "call_sequence")]
    pub call_sequence_raw: Vec<HashMap<String, f64>>,
    #[serde(skip)]
    pub parsed_call_sequence: Vec<Vec<ParsedCall>>,
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
    #[serde(default)]
    pub entry_points: HashMap<String, Vec<HashMap<String, f64>>>,
    #[serde(skip)]
    pub parsed_entry_points: HashMap<String, Vec<Vec<ParsedCall>>>,
    pub services: Vec<ServiceDefinition>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OracleWorkEstimateConfig {
    #[serde(default)]
    pub default_method_overhead_us: u64,
    #[serde(default)]
    pub service_overheads_us: HashMap<String, u64>,
    #[serde(default)]
    pub method_overheads_us: HashMap<String, u64>,
    #[serde(default)]
    pub online_overhead: OnlineOracleOverheadConfig,
}

impl OracleWorkEstimateConfig {
    pub fn overhead_for(&self, service_id: &str, method_name: &str) -> u64 {
        let method_key = format!("{}::{}", service_id, method_name);
        self.default_method_overhead_us
            .saturating_add(*self.service_overheads_us.get(service_id).unwrap_or(&0))
            .saturating_add(*self.method_overheads_us.get(&method_key).unwrap_or(&0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineOracleOverheadPlacement {
    LastCall,
    UniformCalls,
}

impl Default for OnlineOracleOverheadPlacement {
    fn default() -> Self {
        Self::LastCall
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlineOracleOverheadConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_online_overhead_alpha")]
    pub alpha: f64,
    #[serde(default)]
    pub max_overhead_us: u64,
    #[serde(default)]
    pub placement: OnlineOracleOverheadPlacement,
}

impl Default for OnlineOracleOverheadConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            alpha: default_online_overhead_alpha(),
            max_overhead_us: 0,
            placement: OnlineOracleOverheadPlacement::default(),
        }
    }
}

fn default_online_overhead_alpha() -> f64 {
    0.25
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthbenchConfig {
    #[serde(default = "onef64")]
    pub child_cpus_per_replica: f64,
    #[serde(default)]
    pub oracle_work_estimate: OracleWorkEstimateConfig,
    pub call_graph: CallGraphConfig,
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
pub fn validate_call_graph(config: &CallGraphConfig) -> Result<(), String> {
    // Build a set of all valid service::method combinations
    let mut valid_targets = HashMap::new();
    for service in &config.services {
        for method in &service.methods {
            let target = format!("{}::{}", service.id, method.name);
            valid_targets.insert(target, (service.id.clone(), method.name.clone()));
        }
    }

    // Validate entry_points
    if config.entry_points.is_empty() {
        return Err(
            "Call graph must define at least one entry point in 'entry_points'".to_string(),
        );
    }

    for (key, sequence) in &config.entry_points {
        if key != "a" && key != "b" {
            return Err(format!(
                "Invalid entry point key '{}'. Only 'a' and 'b' are allowed.",
                key
            ));
        }
        for step in sequence {
            for (target_str, _prob) in step {
                if !valid_targets.contains_key(target_str) {
                    return Err(format!(
                        "Entry point '{}' references non-existent target '{}'",
                        key, target_str
                    ));
                }
            }
        }
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

    // Parse entry points
    let mut parsed_entry_points = HashMap::new();
    for (key, sequence) in &config.entry_points {
        let mut parsed_sequence = Vec::new();
        for step in sequence {
            let mut parsed_step = Vec::new();
            for (target_str, prob) in step {
                let target = parse_service_method(target_str)?;
                parsed_step.push(ParsedCall::new(target, *prob));
            }
            parsed_sequence.push(parsed_step);
        }
        parsed_entry_points.insert(key.clone(), parsed_sequence);
    }
    config.parsed_entry_points = parsed_entry_points;

    // Parse all call sequences
    for service in &mut config.services {
        for method in &mut service.methods {
            let mut parsed = Vec::new();
            for step in &method.call_sequence_raw {
                let mut parsed_step = Vec::new();
                for (target_str, prob) in step {
                    let target = parse_service_method(target_str)?;
                    parsed_step.push(ParsedCall::new(target, *prob));
                }
                parsed.push(parsed_step);
            }
            method.parsed_call_sequence = parsed;
        }
    }

    #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
    crate::oracle::validate_supported_call_graph(config)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(not(any(feature = "sched_oracle", feature = "eval_oracle_continuation")))]
    use super::parse_call_sequences;
    use super::{
        parse_service_method, validate_call_graph, CallGraphConfig, ServiceDefinition,
        ServiceMethod, SynthbenchConfig,
    };
    #[cfg(feature = "sched_oracle")]
    use super::{
        OnlineOracleOverheadConfig, OnlineOracleOverheadPlacement, OracleWorkEstimateConfig,
    };
    use crate::distribution::LatencyDistribution;
    use rand_distr::Exp;
    use serde_json::json;
    use std::collections::HashMap;

    fn exponential(lambda: f64) -> LatencyDistribution {
        LatencyDistribution::Exponential {
            lambda,
            mean: None,
            dist: Exp::new(lambda).unwrap(),
        }
    }

    #[cfg(feature = "sched_oracle")]
    fn discrete(value: u64) -> LatencyDistribution {
        discrete_values(vec![1.0], vec![value])
    }

    #[cfg(feature = "sched_oracle")]
    fn discrete_values(weights: Vec<f64>, values: Vec<u64>) -> LatencyDistribution {
        LatencyDistribution::Discrete {
            dist: rand_distr::WeightedIndex::new(weights.clone()).unwrap(),
            weights,
            values,
        }
    }

    #[test]
    fn parses_exponential_with_mean() {
        let config = json!({
            "call_graph": {
                "entry_points": {
                    "a": [{"MS_1::method1": 1.0}]
                },
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
                ]
            }
        });

        let parsed: SynthbenchConfig = serde_json::from_value(config).expect("parse config");
        let call_graph = &parsed.call_graph;
        let method = &call_graph.services[0].methods[0];
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
            "call_graph": {
                "entry_points": {
                    "a": [{"MS_1::method1": 1.0}]
                },
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
                ]
            }
        });

        let parsed: SynthbenchConfig = serde_json::from_value(config).expect("parse config");
        let call_graph = &parsed.call_graph;
        let method = &call_graph.services[0].methods[0];
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
            "call_graph": {
                "entry_points": {
                    "a": [{"MS_56394::GqI6UW1mU4": 1.0}]
                },
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

        let parsed: SynthbenchConfig = serde_json::from_value(config).expect("parse config");

        let call_graph = &parsed.call_graph;
        assert_eq!(call_graph.entry_points.len(), 1);
        assert!(call_graph.entry_points.contains_key("a"));
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
        let mut entry_points = HashMap::new();
        entry_points.insert(
            "a".to_string(),
            vec![[("MS_56394::GqI6UW1mU4".to_string(), 1.0)]
                .iter()
                .cloned()
                .collect()],
        );

        let mut config = CallGraphConfig {
            entry_points,
            parsed_entry_points: HashMap::new(),
            services: vec![
                ServiceDefinition {
                    id: "MS_56394".to_string(),
                    replicas: 1,
                    methods: vec![ServiceMethod {
                        name: "GqI6UW1mU4".to_string(),
                        latency_distribution: exponential(0.0001),
                        call_sequence_raw: vec![[("MS_37691::y_DKOh-Gts".to_string(), 1.0)]
                            .iter()
                            .cloned()
                            .collect()],
                        parsed_call_sequence: vec![],
                        busy_spin_ratio: None,
                    }],
                },
                ServiceDefinition {
                    id: "MS_37691".to_string(),
                    replicas: 1,
                    methods: vec![ServiceMethod {
                        name: "y_DKOh-Gts".to_string(),
                        latency_distribution: exponential(0.0001),
                        call_sequence_raw: vec![],
                        parsed_call_sequence: vec![],
                        busy_spin_ratio: None,
                    }],
                },
            ],
        };

        // Should validate successfully
        assert!(validate_call_graph(&config).is_ok());

        // Test invalid entry point key
        config.entry_points.insert("INVALID".to_string(), vec![]);
        assert!(validate_call_graph(&config).is_err());
        config.entry_points.remove("INVALID");

        // Test invalid target reference in entry point
        config.entry_points.get_mut("a").unwrap()[0].insert("INVALID::method".to_string(), 1.0);
        assert!(validate_call_graph(&config).is_err());
        config.entry_points.get_mut("a").unwrap()[0].remove("INVALID::method");

        // Test invalid target reference in service call sequence
        config.services[0].methods[0].call_sequence_raw[0]
            .insert("INVALID::method".to_string(), 1.0);
        assert!(validate_call_graph(&config).is_err());
    }

    #[cfg(not(any(feature = "sched_oracle", feature = "eval_oracle_continuation")))]
    #[test]
    fn test_parse_call_sequences() {
        let mut entry_points = HashMap::new();
        entry_points.insert(
            "a".to_string(),
            vec![[("MS_56394::GqI6UW1mU4".to_string(), 1.0)]
                .iter()
                .cloned()
                .collect()],
        );

        let mut config = CallGraphConfig {
            entry_points,
            parsed_entry_points: HashMap::new(),
            services: vec![
                ServiceDefinition {
                    id: "MS_56394".to_string(),
                    replicas: 1,
                    methods: vec![ServiceMethod {
                        name: "GqI6UW1mU4".to_string(),
                        latency_distribution: exponential(0.0001),
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
                    }],
                },
                ServiceDefinition {
                    id: "MS_37691".to_string(),
                    replicas: 1,
                    methods: vec![
                        ServiceMethod {
                            name: "y_DKOh-Gts".to_string(),
                            latency_distribution: exponential(0.0001),
                            call_sequence_raw: vec![],
                            parsed_call_sequence: vec![],
                            busy_spin_ratio: None,
                        },
                        ServiceMethod {
                            name: "ykccIz2fkK".to_string(),
                            latency_distribution: exponential(0.0001),
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
                        latency_distribution: exponential(0.0001),
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

        // Check that both methods are present in the first step (order doesn't matter due to HashMap)
        let method_names: Vec<&str> = method.parsed_call_sequence[0]
            .iter()
            .map(|call| call.target.method_name.as_str())
            .collect();
        assert!(method_names.contains(&"y_DKOh-Gts"));
        assert!(method_names.contains(&"ykccIz2fkK"));

        // Check service_id and probabilities
        for call in &method.parsed_call_sequence[0] {
            assert_eq!(call.target.service_id, "MS_37691");
            if call.target.method_name == "y_DKOh-Gts" {
                assert_eq!(call.probability, 1.0);
            } else if call.target.method_name == "ykccIz2fkK" {
                assert_eq!(call.probability, 0.8);
            }
        }

        // Check second step
        assert_eq!(
            method.parsed_call_sequence[1][0].target.service_id,
            "MS_73106"
        );
        assert_eq!(
            method.parsed_call_sequence[1][0].target.method_name,
            "method3"
        );
        assert_eq!(method.parsed_call_sequence[1][0].probability, 1.0);
    }

    #[cfg(feature = "sched_oracle")]
    #[test]
    fn test_oracle_planner_adds_realized_work_hints() {
        let mut entry_points = HashMap::new();
        entry_points.insert(
            "a".to_string(),
            vec![
                [("MS_1::first".to_string(), 1.0)].iter().cloned().collect(),
                [("MS_2::second".to_string(), 1.0)]
                    .iter()
                    .cloned()
                    .collect(),
            ],
        );

        let mut config = CallGraphConfig {
            entry_points,
            parsed_entry_points: HashMap::new(),
            services: vec![
                ServiceDefinition {
                    id: "MS_1".to_string(),
                    replicas: 1,
                    methods: vec![ServiceMethod {
                        name: "first".to_string(),
                        latency_distribution: discrete(10_000),
                        call_sequence_raw: vec![],
                        parsed_call_sequence: vec![],
                        busy_spin_ratio: None,
                    }],
                },
                ServiceDefinition {
                    id: "MS_2".to_string(),
                    replicas: 1,
                    methods: vec![ServiceMethod {
                        name: "second".to_string(),
                        latency_distribution: discrete(20_000),
                        call_sequence_raw: vec![],
                        parsed_call_sequence: vec![],
                        busy_spin_ratio: None,
                    }],
                },
            ],
        };

        parse_call_sequences(&mut config).expect("oracle-compatible config should parse");
        let planner =
            crate::oracle::OraclePlanner::new(&config, OracleWorkEstimateConfig::default())
                .expect("planner should build");
        let plan = planner
            .plan_entry_point("a")
            .expect("entrypoint should plan");

        let first_hint = plan.child_steps[0][0].hint;
        assert_eq!(first_hint.child_work_us, 10_000);
        assert_eq!(first_hint.remaining_after_us, 20_000);

        let second_hint = plan.child_steps[1][0].hint;
        assert_eq!(second_hint.child_work_us, 20_000);
        assert_eq!(second_hint.remaining_after_us, 0);
    }

    #[cfg(feature = "sched_oracle")]
    #[test]
    fn test_oracle_planner_accepts_variable_work() {
        let mut entry_points = HashMap::new();
        entry_points.insert(
            "a".to_string(),
            vec![[("MS_1::first".to_string(), 1.0)].iter().cloned().collect()],
        );

        let mut config = CallGraphConfig {
            entry_points,
            parsed_entry_points: HashMap::new(),
            services: vec![ServiceDefinition {
                id: "MS_1".to_string(),
                replicas: 1,
                methods: vec![ServiceMethod {
                    name: "first".to_string(),
                    latency_distribution: discrete_values(vec![1.0, 1.0], vec![10_000, 20_000]),
                    call_sequence_raw: vec![],
                    parsed_call_sequence: vec![],
                    busy_spin_ratio: None,
                }],
            }],
        };

        parse_call_sequences(&mut config).expect("variable work should parse");
        let planner =
            crate::oracle::OraclePlanner::new(&config, OracleWorkEstimateConfig::default())
                .expect("planner should build");
        let plan = planner
            .plan_entry_point("a")
            .expect("entrypoint should plan");
        let realized_work = plan.child_steps[0][0].hint.child_work_us;

        assert!(
            realized_work == 10_000 || realized_work == 20_000,
            "unexpected realized work: {}",
            realized_work
        );
        assert_eq!(plan.child_steps[0][0].hint.remaining_after_us, 0);
        assert_eq!(plan.child_steps[0][0].plan.local_work_us, realized_work);
    }

    #[cfg(feature = "sched_oracle")]
    #[test]
    fn test_oracle_planner_overhead_changes_hints_not_realized_work() {
        let mut entry_points = HashMap::new();
        entry_points.insert(
            "a".to_string(),
            vec![
                [("MS_1::first".to_string(), 1.0)].iter().cloned().collect(),
                [("MS_2::second".to_string(), 1.0)]
                    .iter()
                    .cloned()
                    .collect(),
            ],
        );

        let mut config = CallGraphConfig {
            entry_points,
            parsed_entry_points: HashMap::new(),
            services: vec![
                ServiceDefinition {
                    id: "MS_1".to_string(),
                    replicas: 1,
                    methods: vec![ServiceMethod {
                        name: "first".to_string(),
                        latency_distribution: discrete(10_000),
                        call_sequence_raw: vec![],
                        parsed_call_sequence: vec![],
                        busy_spin_ratio: None,
                    }],
                },
                ServiceDefinition {
                    id: "MS_2".to_string(),
                    replicas: 1,
                    methods: vec![ServiceMethod {
                        name: "second".to_string(),
                        latency_distribution: discrete(20_000),
                        call_sequence_raw: vec![],
                        parsed_call_sequence: vec![],
                        busy_spin_ratio: None,
                    }],
                },
            ],
        };

        let mut estimate = OracleWorkEstimateConfig {
            default_method_overhead_us: 1_000,
            ..Default::default()
        };
        estimate
            .method_overheads_us
            .insert("MS_2::second".to_string(), 4_000);

        parse_call_sequences(&mut config).expect("oracle-compatible config should parse");
        let planner =
            crate::oracle::OraclePlanner::new(&config, estimate).expect("planner should build");
        let plan = planner
            .plan_entry_point("a")
            .expect("entrypoint should plan");

        let first = &plan.child_steps[0][0];
        assert_eq!(first.plan.local_work_us, 10_000);
        assert_eq!(first.hint.child_work_us, 11_000);
        assert_eq!(first.hint.remaining_after_us, 25_000);

        let second = &plan.child_steps[1][0];
        assert_eq!(second.plan.local_work_us, 20_000);
        assert_eq!(second.hint.child_work_us, 25_000);
        assert_eq!(second.hint.remaining_after_us, 0);
    }

    #[cfg(feature = "sched_oracle")]
    #[test]
    fn test_oracle_planner_online_overhead_updates_future_hints() {
        let mut entry_points = HashMap::new();
        entry_points.insert(
            "a".to_string(),
            vec![
                [("MS_1::first".to_string(), 1.0)].iter().cloned().collect(),
                [("MS_2::second".to_string(), 1.0)]
                    .iter()
                    .cloned()
                    .collect(),
            ],
        );

        let mut config = CallGraphConfig {
            entry_points,
            parsed_entry_points: HashMap::new(),
            services: vec![
                ServiceDefinition {
                    id: "MS_1".to_string(),
                    replicas: 1,
                    methods: vec![ServiceMethod {
                        name: "first".to_string(),
                        latency_distribution: discrete(10_000),
                        call_sequence_raw: vec![],
                        parsed_call_sequence: vec![],
                        busy_spin_ratio: None,
                    }],
                },
                ServiceDefinition {
                    id: "MS_2".to_string(),
                    replicas: 1,
                    methods: vec![ServiceMethod {
                        name: "second".to_string(),
                        latency_distribution: discrete(20_000),
                        call_sequence_raw: vec![],
                        parsed_call_sequence: vec![],
                        busy_spin_ratio: None,
                    }],
                },
            ],
        };

        parse_call_sequences(&mut config).expect("oracle-compatible config should parse");
        let estimate = OracleWorkEstimateConfig {
            online_overhead: OnlineOracleOverheadConfig {
                enabled: true,
                alpha: 1.0,
                max_overhead_us: 0,
                placement: OnlineOracleOverheadPlacement::LastCall,
            },
            ..Default::default()
        };
        let planner =
            crate::oracle::OraclePlanner::new(&config, estimate).expect("planner should build");

        let initial = planner
            .plan_entry_point("a")
            .expect("entrypoint should plan");
        assert_eq!(initial.child_steps[0][0].hint.remaining_after_us, 20_000);

        planner.record_entry_point_latency("a", 40_000, &initial);

        let calibrated = planner
            .plan_entry_point("a")
            .expect("entrypoint should plan");
        assert_eq!(calibrated.child_steps[0][0].hint.remaining_after_us, 30_000);
        assert_eq!(calibrated.child_steps[1][0].hint.child_work_us, 30_000);
        assert_eq!(calibrated.child_steps[1][0].plan.local_work_us, 20_000);
    }
}
