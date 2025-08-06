use app_utils::timing::time_now;
use rand::thread_rng;
use rand_distr::{Distribution, Normal, WeightedIndex};

use crate::config;

pub struct Hop {
    pub server: usize,
    pub sleep: bool,
    pub latency_distribution: LatencyDistribution,
}

impl From<config::Hop> for Hop {
    fn from(value: config::Hop) -> Self {
        Self {
            server: value.server,
            sleep: value.sleep,
            latency_distribution: LatencyDistribution::from(value.latency_distribution),
        }
    }
}

pub enum LatencyDistribution {
    Normal(Normal<f64>),
    Discrete(WeightedIndex<f64>, Vec<u64>),
    Periodic {
        slow_latency: u64,
        fast_latency: u64,
        slow_duration_ms: u16,
    },
}

impl LatencyDistribution {
    // returns latency in us
    pub fn sample(&self) -> u64 {
        match self {
            LatencyDistribution::Normal(d) => d.sample(&mut thread_rng()).round() as u64,
            LatencyDistribution::Discrete(d, values) => values[d.sample(&mut thread_rng())],
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
}

impl From<config::LatencyDistribution> for LatencyDistribution {
    fn from(value: config::LatencyDistribution) -> Self {
        match value {
            config::LatencyDistribution::Normal { mean, std } => {
                LatencyDistribution::Normal(Normal::new(mean, std).unwrap())
            }
            config::LatencyDistribution::Discrete { weights, values } => {
                assert_eq!(weights.len(), values.len());
                LatencyDistribution::Discrete(WeightedIndex::new(weights).unwrap(), values)
            }
            config::LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            },
        }
    }
}
