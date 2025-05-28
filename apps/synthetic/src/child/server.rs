pub mod synthetic_tonic {
    pub mod child {
        tonic::include_proto!("child");
    }
}

use rand_distr::{Distribution, Normal};
use std::time::{Duration, Instant};

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
                // convert from us to ns
                (config.child_random_mean * 1000) as f64,
                (config.child_random_std * 1000) as f64,
            )
            .unwrap(),
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
        _request: Request<child::ConstantLatencyRequest>,
    ) -> Result<Response<child::ConstantLatencyResponse>, Status> {
        let start = Instant::now();

        do_work(Duration::from_micros(self.constant_latency));

        Ok(Response::new(child::ConstantLatencyResponse {
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }

    async fn random_latency(
        &self,
        _request: Request<child::RandomLatencyRequest>,
    ) -> Result<Response<child::RandomLatencyResponse>, Status> {
        let start = Instant::now();

        let duration = Duration::from_nanos(
            self.random_latency_distribution
                .sample(&mut rand::thread_rng()) as u64,
        );
        do_work(duration);

        Ok(Response::new(child::RandomLatencyResponse {
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }
}
