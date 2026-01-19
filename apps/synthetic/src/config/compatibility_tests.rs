#[cfg(test)]
mod tests {
    use serde_json;

    use crate::config::types::{LatencyDistribution, ServiceMethod, SyntheticConfig};

    #[test]
    fn test_backward_compatibility_minimal_config() {
        // Test that minimal config still works
        let json = r#"{}"#;
        let config: SyntheticConfig = serde_json::from_str(json).unwrap();

        // Should have default values
        assert!(matches!(
            config.child_random_latency,
            LatencyDistribution::Exponential { .. }
        ));
        assert!(config.child_services.is_empty());
        assert!(config.request_a_hops.is_empty());
        assert!(config.request_b_hops.is_empty());
        assert!(config.call_graph.is_none());
    }

    #[test]
    fn test_backward_compatibility_traditional_config() {
        let json = r#"{
            "child_services": [
                {"id": "S1", "replicas": 2},
                {"id": "C6", "replicas": 1}
            ],
            "request_a_hops": [
                {"service_id": "S1", "duration_us": 12000},
                {"service_id": "C6", "duration_us": 8000}
            ],
            "request_b_hops": [
                {"service_id": "S1", "duration_us": 5000}
            ]
        }"#;

        let config: SyntheticConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.child_services.len(), 2);
        assert_eq!(config.child_services[0].id, "S1");
        assert_eq!(config.child_services[0].replicas, 2);
        assert_eq!(config.child_services[1].id, "C6");
        assert_eq!(config.child_services[1].replicas, 1);

        assert_eq!(config.request_a_hops.len(), 2);
        assert_eq!(config.request_a_hops[0].service_id, "S1");
        assert_eq!(config.request_a_hops[0].duration_us, Some(12000));

        assert_eq!(config.request_b_hops.len(), 1);
        assert_eq!(config.request_b_hops[0].service_id, "S1");
        assert_eq!(config.request_b_hops[0].duration_us, Some(5000));
    }

    #[test]
    fn test_backward_compatibility_call_graph_config() {
        let json = r#"{
            "call_graph": {
                "entry_point": "MS_1::method1",
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
        }"#;

        let config: SyntheticConfig = serde_json::from_str(json).unwrap();

        assert!(config.call_graph.is_some());
        let call_graph = config.call_graph.unwrap();
        assert_eq!(call_graph.entry_point, "MS_1::method1");
        assert_eq!(call_graph.services.len(), 1);
        assert_eq!(call_graph.services[0].id, "MS_1");
        assert_eq!(call_graph.services[0].methods.len(), 1);
        assert_eq!(call_graph.services[0].methods[0].name, "method1");
    }

    #[test]
    fn test_latency_distribution_serialization() {
        let original = LatencyDistribution::Periodic {
            slow_latency: 10000,
            fast_latency: 1000,
            slow_duration_ms: 500,
        };

        let json = serde_json::to_string(&original).unwrap();
        let deserialized: LatencyDistribution = serde_json::from_str(&json).unwrap();

        match deserialized {
            LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => {
                assert_eq!(slow_latency, 10000);
                assert_eq!(fast_latency, 1000);
                assert_eq!(slow_duration_ms, 500);
            }
            _ => panic!("Expected Periodic distribution"),
        }
    }

    #[test]
    fn test_exponential_distribution_with_mean() {
        let json = r#"{"Exponential": {"mean": 20000.0}}"#;
        let distribution: LatencyDistribution = serde_json::from_str(json).unwrap();

        match distribution {
            LatencyDistribution::Exponential { mean, lambda, .. } => {
                assert_eq!(mean, Some(20000.0));
                // mean = 20000 should give lambda = 1/20000 = 0.00005
                assert!((lambda - 0.00005).abs() < 1e-10);
            }
            _ => panic!("Expected Exponential distribution"),
        }
    }

    #[test]
    fn test_exponential_distribution_with_lambda() {
        let json = r#"{"Exponential": {"lambda": 0.0001}}"#;
        let distribution: LatencyDistribution = serde_json::from_str(json).unwrap();

        match distribution {
            LatencyDistribution::Exponential { lambda, mean, .. } => {
                assert_eq!(lambda, 0.0001);
                assert_eq!(mean, None);
            }
            _ => panic!("Expected Exponential distribution"),
        }
    }

    #[test]
    fn test_exponential_distribution_prefers_mean() {
        let json = r#"{"Exponential": {"lambda": 0.0002, "mean": 10000.0}}"#;
        let distribution: LatencyDistribution = serde_json::from_str(json).unwrap();

        match distribution {
            LatencyDistribution::Exponential { lambda, mean, .. } => {
                // Should prefer mean, so lambda should be 1/10000 = 0.0001, not 0.0002
                assert!((lambda - 0.0001).abs() < 1e-10);
                assert_eq!(mean, Some(10000.0));
            }
            _ => panic!("Expected Exponential distribution"),
        }
    }

    #[test]
    fn test_normal_distribution() {
        let json = r#"{"Normal": {"mean": 5000.0, "std": 1000.0}}"#;
        let distribution: LatencyDistribution = serde_json::from_str(json).unwrap();

        match distribution {
            LatencyDistribution::Normal { mean, std, .. } => {
                assert_eq!(mean, 5000.0);
                assert_eq!(std, 1000.0);
            }
            _ => panic!("Expected Normal distribution"),
        }
    }

    #[test]
    fn test_discrete_distribution() {
        let json = r#"{"Discrete": {"weights": [0.3, 0.7], "values": [1000, 5000]}}"#;
        let distribution: LatencyDistribution = serde_json::from_str(json).unwrap();

        match distribution {
            LatencyDistribution::Discrete {
                weights, values, ..
            } => {
                assert_eq!(weights, vec![0.3, 0.7]);
                assert_eq!(values, vec![1000, 5000]);
            }
            _ => panic!("Expected Discrete distribution"),
        }
    }

    #[test]
    fn test_service_method_validation() {
        let method_json = r#"{
            "name": "test_method",
            "latency_distribution": {"Exponential": {"mean": 5000.0}},
            "call_sequence": [
                {"service1::method1": 1.0}
            ]
        }"#;

        let method: ServiceMethod = serde_json::from_str(method_json).unwrap();
        assert_eq!(method.name, "test_method");
        assert_eq!(method.call_sequence_raw.len(), 1);
        assert_eq!(method.call_sequence_raw[0].len(), 1);
        assert!(method.call_sequence_raw[0].contains_key("service1::method1"));
    }

    #[test]
    fn test_default_random_latency() {
        let config: SyntheticConfig = serde_json::from_str("{}").unwrap();

        match config.child_random_latency {
            LatencyDistribution::Exponential { lambda, mean, .. } => {
                // Default should be exponential with mean 10000 (lambda = 1/10000)
                assert!((lambda - 0.0001).abs() < 1e-10);
                assert_eq!(mean, Some(10000.0));
            }
            _ => panic!("Expected default exponential distribution"),
        }
    }
}
