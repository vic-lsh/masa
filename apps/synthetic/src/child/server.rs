use std::time::{Duration, Instant};

use tokio;
use tonic::{Request, Response, Status};

use app_utils::timing::time_now;
use synthetic_app::config::SyntheticConfig;
use synthetic_app::tonic::{child, child::child_server::Child};
use synthetic_app::util;

use synthetic_app::tonic::child::Periodic;

pub struct ChildImpl {
    constant_latency: util::LatencyDistribution,
    random_latency: util::LatencyDistribution,
}

impl ChildImpl {
    pub fn new(config: SyntheticConfig) -> Self {
        assert!(config.child_constant_latency_slowdown_duration <= 1000);

        let random_latency = util::LatencyDistribution::from(config.child_random_latency);

        ChildImpl {
            constant_latency: util::LatencyDistribution::Periodic {
                fast_latency: config.child_constant_latency,
                slow_latency: 2 * config.child_constant_latency,
                slow_duration_ms: config.child_constant_latency_slowdown_duration,
            },
            random_latency,
        }
    }
}

fn busy_spin(duration: Duration) {
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

        let duration = Duration::from_micros(self.constant_latency.sample());

        busy_spin(duration);

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
        let duration = Duration::from_millis(duration_us / 1_000);
        tokio::time::sleep(duration).await;

        Ok(Response::new(child::RandomLatencyResponse {
            queueing_latency,
            sleep_latency: duration.as_micros() as u64,
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }

    async fn presampled(
        &self,
        request: Request<child::PresampledRequest>,
    ) -> Result<Response<child::PresampledResponse>, Status> {
        let request = request.into_inner();

        let total_duration = match request.latency.unwrap().latency_type.unwrap() {
            child::latency::LatencyType::Fixed(fixed) => fixed.latency,
            child::latency::LatencyType::Periodic(Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            }) => util::LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms: slow_duration_ms as u16,
            }
            .sample(),
        };
        let sleep_duration = (request.sleep * total_duration as f64).round() as u64;
        let spin_duration = total_duration - sleep_duration;

        if spin_duration > 0 {
            busy_spin(Duration::from_micros(spin_duration));
        }

        // TODO: might want to update slack here

        if sleep_duration > 0 {
            tokio::time::sleep(Duration::from_micros(sleep_duration)).await;
        }

        // TODO: ... and here

        Ok(Response::new(child::PresampledResponse {}))
    }
}
