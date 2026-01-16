pub mod synthetic_tonic {
    pub mod child {
        tonic::include_proto!("child");
    }
}

use std::time::{Duration, Instant};

use tokio;
use tokio::runtime::current_thread_queue_len;
use tonic::{Request, Response, Status};

use crate::server::synthetic_tonic::child::Periodic;
use app_utils::timing::time_now;
use synthetic::config::SyntheticConfig;
use synthetic::util;
use synthetic_tonic::{child, child::child_server::Child};

pub struct ChildImpl {
    random_latency: util::LatencyDistribution,
}

impl ChildImpl {
    pub fn new(config: SyntheticConfig) -> Self {
        let random_latency = util::LatencyDistribution::from(config.child_random_latency);

        // Spawn a task that prints the queue length every 500ms
        tokio::spawn(async {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            let start_time = Instant::now();
            loop {
                interval.tick().await;
                let queue_len = current_thread_queue_len();
                let elapsed = start_time.elapsed();
                println!(
                    "current_thread_queue_len: {} (elapsed: {:?})",
                    queue_len, elapsed
                );
            }
        });

        ChildImpl { random_latency }
    }
}

fn busy_spin(duration: Duration) {
    let end = Instant::now() + duration;

    while Instant::now() < end {}
}

#[tonic::async_trait]
impl Child for ChildImpl {
    async fn run_synthetic(
        &self,
        request: Request<child::RunSyntheticRequest>,
    ) -> Result<Response<child::RunSyntheticResponse>, Status> {
        let request = request.into_inner();
        let queueing_latency = time_now() - request.sent_at;
        let start = Instant::now();

        let total_duration_us = match request.duration_us {
            Some(duration) => duration,
            None => self.random_latency.sample(),
        };

        let busy_spin_dur_us = request.busy_spin_dur_us.unwrap_or(0);

        // Validate that busy_spin duration is not greater than total duration
        if busy_spin_dur_us > total_duration_us {
            return Err(Status::invalid_argument(format!(
                "busy_spin_dur_us ({}) cannot be greater than total duration_us ({})",
                busy_spin_dur_us, total_duration_us
            )));
        }

        let sleep_dur_us = total_duration_us - busy_spin_dur_us;

        // Sleep first
        if sleep_dur_us > 0 {
            tokio::time::sleep(Duration::from_micros(sleep_dur_us)).await;
        }

        // Then busy spin
        if busy_spin_dur_us > 0 {
            let yield_interval = Duration::from_micros(200);
            let busy_spin_duration = Duration::from_micros(busy_spin_dur_us);

            let mut remaining = busy_spin_duration;
            while remaining > yield_interval {
                busy_spin(yield_interval);
                tokio::task::yield_now().await;
                remaining -= yield_interval;
            }
            if remaining > Duration::ZERO {
                busy_spin(remaining);
            }
        }

        Ok(Response::new(child::RunSyntheticResponse {
            queueing_latency,
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
            finished_at: time_now(),
        }))
    }

    async fn random_latency(
        &self,
        request: Request<child::RandomLatencyRequest>,
    ) -> Result<Response<child::RandomLatencyResponse>, Status> {
        let request = request.into_inner();
        let queueing_latency = time_now() - request.sent_at;
        let start = Instant::now();

        let duration_us = self.random_latency.sample();
        // sleep has millisecond granularity so we round the duration time

        let duration = Duration::from_micros(duration_us);
        if request.busy_spin {
            let yield_interval = Duration::from_micros(200);

            let mut remaining = duration;
            while remaining > yield_interval {
                busy_spin(yield_interval);
                tokio::task::yield_now().await;
                remaining -= yield_interval;
            }
            if remaining > Duration::ZERO {
                busy_spin(remaining);
            }
        } else {
            tokio::time::sleep(duration).await;
        }

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
