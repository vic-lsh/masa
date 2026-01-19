use rand::{thread_rng, Rng};

use crate::config;

pub struct Hop {
    pub service: usize,
    pub sleep: f64,
    pub latency_distribution: config::LatencyDistribution,
}

impl From<config::Hop> for Hop {
    fn from(value: config::Hop) -> Self {
        Self {
            service: value.service,
            sleep: value.sleep,
            latency_distribution: value.latency_distribution,
        }
    }
}

/// Sample whether a call should be made based on probability.
/// Returns true if a random value [0.0, 1.0) is less than the given probability.
pub fn should_make_call(probability: f64) -> bool {
    let mut rng = thread_rng();
    rng.gen::<f64>() < probability
}
