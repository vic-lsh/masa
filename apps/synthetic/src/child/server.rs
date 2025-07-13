pub mod synthetic_tonic {
    pub mod child {
        tonic::include_proto!("child");
    }
}

use rand::thread_rng;
use rand_distr::{Distribution, Normal, WeightedIndex};
use std::time::{Duration, Instant};

use tokio;
use tonic::{Request, Response, Status};

use crate::config;
use crate::config::SyntheticConfig;
use app_utils::timing::time_now;
use synthetic_tonic::{child, child::child_server::Child};

enum LatencyDistribution {
    Normal(Normal<f64>),
    Discrete(WeightedIndex<f64>, Vec<f64>),
}

impl LatencyDistribution {
    fn sample(&self) -> f64 {
        match self {
            LatencyDistribution::Normal(d) => d.sample(&mut thread_rng()),
            LatencyDistribution::Discrete(d, values) => values[d.sample(&mut thread_rng())],
        }
    }
}

pub struct ChildImpl {
    constant_latency: u64,
    constant_latency_slowdown_duration: u16,
    random_latency: LatencyDistribution,
}

impl ChildImpl {
    pub fn new(config: SyntheticConfig) -> Self {
        assert!(config.child_constant_latency_slowdown_duration <= 1000);

        let random_latency = match config.child_random_latency {
            config::LatencyDistribution::Normal { mean, std } => {
                LatencyDistribution::Normal(Normal::new(mean, std).unwrap())
            }
            config::LatencyDistribution::Discrete { weights, values } => {
                assert_eq!(weights.len(), values.len());
                LatencyDistribution::Discrete(WeightedIndex::new(weights).unwrap(), values)
            }
        };

        ChildImpl {
            constant_latency: config.child_constant_latency,
            constant_latency_slowdown_duration: config.child_constant_latency_slowdown_duration,
            random_latency,
        }
    }

    // slow down for constant_latency_slowdown_duration ms every second
    fn get_constant_latency(&self) -> u64 {
        let now_ms = (time_now() / 1000) % 1000;

        if now_ms < self.constant_latency_slowdown_duration as u64 {
            self.constant_latency * 2
        } else {
            self.constant_latency
        }
    }
}

// busy spin duration time
fn do_work(duration: Duration) {
    let end = Instant::now() + duration;

    while Instant::now() < end {}
}

#[tonic::async_trait]
impl Child for ChildImpl {
    async fn constant_latency(
        &self,
        request: Request<child::ConstantLatencyRequest>,
    ) -> Result<Response<child::ConstantLatencyResponse>, Status> {
        let queueing_latency = time_now() - request.into_inner().sent_at;
        let start = Instant::now();

        let duration = Duration::from_micros(self.get_constant_latency());

        do_work(duration);

        Ok(Response::new(child::ConstantLatencyResponse {
            queueing_latency,
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
            finished_at: time_now(),
        }))
    }

    async fn random_latency(
        &self,
        request: Request<child::RandomLatencyRequest>,
    ) -> Result<Response<child::RandomLatencyResponse>, Status> {
        let queueing_latency = time_now() - request.into_inner().sent_at;
        let start = Instant::now();

        let duration_us = self.random_latency.sample();
        // sleep has millisecond granularity so we round the duration time
        let duration = Duration::from_millis((duration_us / 1_000.0).round() as u64);
        tokio::time::sleep(duration).await;

        Ok(Response::new(child::RandomLatencyResponse {
            queueing_latency,
            sleep_latency: duration.as_micros() as u64,
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }
}
