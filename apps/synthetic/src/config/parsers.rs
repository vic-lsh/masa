// JSON parsing and serialization logic

use rand_distr::{Exp, Normal, WeightedIndex};
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    distributions::LatencyDistribution,
    types::CallGraphConfig,
    validation::{parse_service_method, validate_call_graph},
};
use crate::error::{ConfigError, Result};

impl Serialize for LatencyDistribution {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        enum LatencyDistributionSer {
            Normal {
                mean: f64,
                std: f64,
            },
            Exponential {
                lambda: f64,
                #[serde(skip_serializing_if = "Option::is_none")]
                mean: Option<f64>,
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

        let ser = match self {
            LatencyDistribution::Normal { mean, std, .. } => LatencyDistributionSer::Normal {
                mean: *mean,
                std: *std,
            },
            LatencyDistribution::Exponential { lambda, mean, .. } => {
                LatencyDistributionSer::Exponential {
                    lambda: *lambda,
                    mean: *mean,
                }
            }
            LatencyDistribution::Discrete {
                weights, values, ..
            } => LatencyDistributionSer::Discrete {
                weights: weights.clone(),
                values: values.clone(),
            },
            LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => LatencyDistributionSer::Periodic {
                slow_latency: *slow_latency,
                fast_latency: *fast_latency,
                slow_duration_ms: *slow_duration_ms,
            },
        };
        ser.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LatencyDistribution {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum LatencyDistributionDe {
            Normal {
                mean: f64,
                std: f64,
            },
            Exponential {
                #[serde(default)]
                lambda: Option<f64>,
                #[serde(default)]
                mean: Option<f64>,
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

        let de = LatencyDistributionDe::deserialize(deserializer)?;
        Ok(match de {
            LatencyDistributionDe::Normal { mean, std } => {
                let dist = Normal::new(mean, std).map_err(|e| {
                    serde::de::Error::custom(format!("Invalid normal distribution: {}", e))
                })?;
                LatencyDistribution::Normal { mean, std, dist }
            }
            LatencyDistributionDe::Exponential { lambda, mean } => {
                let (lambda_val, mean_val) = match (lambda, mean) {
                    (Some(_l), Some(m)) => {
                        // If both provided, prefer mean
                        let lambda_from_mean = 1.0 / m;
                        (lambda_from_mean, Some(m))
                    }
                    (Some(l), None) => (l, None),
                    (None, Some(m)) => {
                        if m <= 0.0 {
                            return Err(serde::de::Error::custom(
                                "Exponential mean must be positive",
                            ));
                        }
                        (1.0 / m, Some(m))
                    }
                    (None, None) => {
                        return Err(serde::de::Error::custom(
                            "Exponential distribution requires either 'lambda' or 'mean' parameter",
                        ));
                    }
                };

                if lambda_val <= 0.0 {
                    return Err(serde::de::Error::custom(
                        "Exponential lambda must be positive",
                    ));
                }

                let dist = Exp::new(lambda_val).map_err(|e| {
                    serde::de::Error::custom(format!("Invalid exponential distribution: {}", e))
                })?;

                LatencyDistribution::Exponential {
                    lambda: lambda_val,
                    mean: mean_val,
                    dist,
                }
            }
            LatencyDistributionDe::Discrete { weights, values } => {
                if weights.len() != values.len() {
                    return Err(serde::de::Error::custom(
                        "Discrete distribution: weights and values must have same length",
                    ));
                }

                if weights.is_empty() {
                    return Err(serde::de::Error::custom(
                        "Discrete distribution: weights and values cannot be empty",
                    ));
                }

                let dist = WeightedIndex::new(&weights).map_err(|e| {
                    serde::de::Error::custom(format!("Invalid discrete distribution: {}", e))
                })?;

                LatencyDistribution::Discrete {
                    weights,
                    values,
                    dist,
                }
            }
            LatencyDistributionDe::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => {
                if slow_duration_ms == 0 || slow_duration_ms > 1000 {
                    return Err(serde::de::Error::custom(
                        "Periodic slow_duration_ms must be between 1 and 1000",
                    ));
                }

                LatencyDistribution::Periodic {
                    slow_latency,
                    fast_latency,
                    slow_duration_ms,
                }
            }
        })
    }
}

/// Parse and validate call sequences for all methods in a call graph config
pub fn parse_call_sequences(config: &mut CallGraphConfig) -> Result<()> {
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
                    if prob < &0.0 || prob > &1.0 {
                        return Err(ConfigError::InvalidCallGraph(format!(
                            "Invalid probability {} for call to {}",
                            prob, target_str
                        ))
                        .into());
                    }
                    parsed_step.push((target, *prob));
                }
                parsed.push(parsed_step);
            }
            method.parsed_call_sequence = parsed;
        }
    }

    Ok(())
}
