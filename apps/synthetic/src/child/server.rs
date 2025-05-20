pub mod synthetic_tonic {
    pub mod child {
        tonic::include_proto!("child");
    }
}

use rand_distr::{Distribution, Normal};
use std::time::Duration;

use tonic::{Request, Response, Status};

use crate::config::SyntheticConfig;
use synthetic_tonic::{child, child::child_server::Child};

pub struct ChildImpl {
    constant_latency: u64,
    // mean and std are float values in nanoseconds
    random_latency_distribution: Normal<f64>,
}

impl ChildImpl {
    pub fn new(config: SyntheticConfig) -> Self {
        ChildImpl {
            constant_latency: config.child_constant_latency,
            random_latency_distribution: Normal::new(
                (config.child_random_mean * 1000) as f64,
                (config.child_random_std * 1000) as f64,
            )
            .unwrap(),
        }
    }
}

#[tonic::async_trait]
impl Child for ChildImpl {
    async fn constant_latency(
        &self,
        _request: Request<child::ConstantLatencyRequest>,
    ) -> Result<Response<child::ConstantLatencyResponse>, Status> {
        tokio::time::sleep(Duration::from_micros(self.constant_latency)).await;

        Ok(Response::new(child::ConstantLatencyResponse {}))
    }

    async fn random_latency(
        &self,
        _request: Request<child::RandomLatencyRequest>,
    ) -> Result<Response<child::RandomLatencyResponse>, Status> {
        let sleep_duration = Duration::from_nanos(
            self.random_latency_distribution
                .sample(&mut rand::thread_rng()) as u64,
        );
        tokio::time::sleep(sleep_duration).await;

        Ok(Response::new(child::RandomLatencyResponse {}))
    }
}
