// Latency distribution implementations

use crate::constants::*;
use crate::error::{ConfigError, Result};
use crate::tonic::child::{latency::LatencyType, Fixed, Periodic};
use app_utils::timing::time_now;
use rand::thread_rng;
use rand_distr::{Distribution, Exp, Normal, WeightedIndex};

#[derive(Debug)]
pub enum LatencyDistribution {
    Normal {
        mean: f64,
        std: f64,
        dist: Normal<f64>,
    },
    Exponential {
        lambda: f64,
        mean: Option<f64>,
        dist: Exp<f64>,
    },
    Discrete {
        weights: Vec<f64>,
        values: Vec<u64>,
        dist: WeightedIndex<f64>,
    },
    Periodic {
        slow_latency: u64,
        fast_latency: u64,
        slow_duration_ms: u16,
    },
}

impl LatencyDistribution {
    /// Sample a latency value in microseconds
    pub fn sample(&self) -> u64 {
        match self {
            LatencyDistribution::Normal { dist, .. } => {
                let mut l = dist.sample(&mut thread_rng()).round();
                if l < 0.0 {
                    l = 0.0;
                }
                l as u64
            }
            LatencyDistribution::Exponential { dist, .. } => {
                dist.sample(&mut thread_rng()).round() as u64
            }
            LatencyDistribution::Discrete { dist, values, .. } => {
                values[dist.sample(&mut thread_rng())]
            }
            LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => {
                let now_ms = (time_now() / 1000) % 1000;
                if now_ms < *slow_duration_ms as u64 {
                    *slow_latency
                } else {
                    *fast_latency
                }
            }
        }
    }

    /// Create a presampled latency for gRPC transmission
    pub fn presample(&self) -> crate::tonic::child::Latency {
        let latency = match self {
            LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => LatencyType::Periodic(Periodic {
                slow_latency: *slow_latency,
                fast_latency: *fast_latency,
                slow_duration_ms: *slow_duration_ms as u32,
            }),
            _ => LatencyType::Fixed(Fixed {
                latency: self.sample(),
            }),
        };

        crate::tonic::child::Latency {
            latency_type: Some(latency),
        }
    }

    /// Validate the distribution parameters
    pub fn validate(&self) -> Result<()> {
        match self {
            LatencyDistribution::Normal { mean, std, .. } => {
                if *std <= 0.0 {
                    return Err(ConfigError::InvalidNormal(
                        "Standard deviation must be positive".to_string(),
                    )
                    .into());
                }
                if *mean < 0.0 {
                    return Err(ConfigError::InvalidNormal(
                        "Mean must be non-negative".to_string(),
                    )
                    .into());
                }
            }
            LatencyDistribution::Exponential { lambda, .. } => {
                if *lambda <= 0.0 {
                    return Err(ConfigError::InvalidExponential(
                        "Lambda must be positive".to_string(),
                    )
                    .into());
                }
            }
            LatencyDistribution::Discrete {
                weights, values, ..
            } => {
                if weights.is_empty() || values.is_empty() {
                    return Err(ConfigError::InvalidDiscrete(
                        "Weights and values cannot be empty".to_string(),
                    )
                    .into());
                }
                if weights.len() != values.len() {
                    return Err(ConfigError::InvalidDiscrete(
                        "Weights and values must have the same length".to_string(),
                    )
                    .into());
                }
                if weights.iter().any(|w| *w <= 0.0) {
                    return Err(ConfigError::InvalidDiscrete(
                        "All weights must be positive".to_string(),
                    )
                    .into());
                }
            }
            LatencyDistribution::Periodic {
                slow_duration_ms, ..
            } => {
                if *slow_duration_ms == 0 || *slow_duration_ms > 1000 {
                    return Err(ConfigError::InvalidPeriodic(
                        "Slow duration must be between 1 and 1000 ms".to_string(),
                    )
                    .into());
                }
            }
        }
        Ok(())
    }
}

impl Clone for LatencyDistribution {
    fn clone(&self) -> Self {
        match self {
            LatencyDistribution::Normal { mean, std, .. } => {
                let dist = Normal::new(*mean, *std)
                    .map_err(|e| {
                        ConfigError::InvalidNormal(format!("Failed to create distribution: {}", e))
                    })
                    .unwrap();
                LatencyDistribution::Normal {
                    mean: *mean,
                    std: *std,
                    dist,
                }
            }
            LatencyDistribution::Exponential { lambda, mean, .. } => {
                let dist = Exp::new(*lambda)
                    .map_err(|e| {
                        ConfigError::InvalidExponential(format!(
                            "Failed to create distribution: {}",
                            e
                        ))
                    })
                    .unwrap();
                LatencyDistribution::Exponential {
                    lambda: *lambda,
                    mean: *mean,
                    dist,
                }
            }
            LatencyDistribution::Discrete {
                weights, values, ..
            } => {
                let dist = WeightedIndex::new(weights)
                    .map_err(|e| {
                        ConfigError::InvalidDiscrete(format!(
                            "Failed to create distribution: {}",
                            e
                        ))
                    })
                    .unwrap();
                LatencyDistribution::Discrete {
                    weights: weights.clone(),
                    values: values.clone(),
                    dist,
                }
            }
            LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => LatencyDistribution::Periodic {
                slow_latency: *slow_latency,
                fast_latency: *fast_latency,
                slow_duration_ms: *slow_duration_ms,
            },
        }
    }
}

/// Create default exponential distribution
pub fn default_random_latency() -> LatencyDistribution {
    let lambda = 1.0 / DEFAULT_EXPONENTIAL_LATENCY_US as f64;
    let dist = Exp::new(lambda).unwrap();
    LatencyDistribution::Exponential {
        lambda,
        mean: Some(DEFAULT_EXPONENTIAL_LATENCY_US as f64),
        dist,
    }
}
