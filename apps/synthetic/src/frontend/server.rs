pub mod synthetic_tonic {
    pub mod frontend {
        tonic::include_proto!("frontend");
    }

    pub mod child {
        tonic::include_proto!("child");
    }
}

use std::collections::HashMap;
use std::iter::zip;
use std::{
    sync::atomic::{AtomicU8, Ordering},
    time::Instant,
};

use crate::config::SyntheticConfig;
use crate::util;
use app_utils::channel::LoadBalancedChannel;
use app_utils::timing::time_now;

use tonic::{Request, Response, Status};

use synthetic_tonic::{
    child, child::child_client::ChildClient, child::Fixed, child::Periodic, frontend,
    frontend::frontend_server::Frontend,
};

pub struct FrontendImpl {
    children: Vec<ChildClient<LoadBalancedChannel>>,
    constant_replicas: u8,
    next_constant_replica: AtomicU8,
    presampled_services: usize,
    presampled_services_offset: usize,
    presampled_request_types: HashMap<String, Vec<util::Hop>>,
}

impl FrontendImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let presampled_services = config.child_presampled_services.len();
        let mut services = vec![1];
        services.extend(vec![config.child_constant_replicas]);
        let presampled_services_offset = services.len();
        services.extend(config.child_presampled_services);
        let mut children = Vec::new();
        for r in services {
            let hostname_base = "local-child-service";
            children.push(ChildClient::new(
                LoadBalancedChannel::new(hostname_base.to_string(), 8000, r).await,
            ));
        }

        let mut presampled_request_types = HashMap::new();
        for (key, value) in config.child_presampled_request_types {
            presampled_request_types.insert(
                key,
                value.into_iter().map(|hop| util::Hop::from(hop)).collect(),
            );
        }

        FrontendImpl {
            children,
            constant_replicas: config.child_constant_replicas,
            next_constant_replica: AtomicU8::new(0),
            presampled_request_types,
            presampled_services,
            presampled_services_offset,
        }
    }
}

#[tonic::async_trait]
impl Frontend for FrontendImpl {
    async fn handle_ping(
        &self,
        _request: Request<frontend::PingRequest>,
    ) -> Result<Response<frontend::PingResponse>, Status> {
        let response = frontend::PingResponse {
            message: "pong".to_string(),
        };
        let response = Response::new(response);
        Ok(response)
    }

    async fn handle_a(
        &self,
        _request: Request<frontend::ARequest>,
    ) -> Result<Response<frontend::AResponse>, Status> {
        let start = Instant::now();
        let mut child_random_client = self.children.last().unwrap().clone();
        let response = child_random_client
            .random_latency(child::RandomLatencyRequest {
                sent_at: time_now(),
            })
            .await?;
        let child_random_response = response.into_inner();

        let next =
            self.next_constant_replica.fetch_add(1, Ordering::SeqCst) % self.constant_replicas;
        let mut child_constant_client = self.children[next as usize].clone();
        let response = child_constant_client
            .constant_latency(child::ConstantLatencyRequest {
                sent_at: time_now(),
            })
            .await?;
        let child_constant_response = response.into_inner();

        Ok(Response::new(frontend::AResponse {
            child1_queueing_latency: child_random_response.queueing_latency,
            child1_sleep_latency: child_random_response.sleep_latency,
            child1_handler_latency: child_random_response.handler_latency,
            child2_queueing_latency: child_constant_response.queueing_latency,
            child2_handler_latency: child_constant_response.handler_latency,
            child2_reply_latency: time_now() - child_constant_response.finished_at,
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }

    async fn handle_presampled(
        &self,
        request: Request<frontend::PresampledRequest>,
    ) -> Result<Response<frontend::PresampledResponse>, Status> {
        // let start = Instant::now();

        let request_type = request.into_inner().request_type;
        let hops = self.presampled_request_types.get(&request_type).unwrap();
        // sample latencies
        let latencies = hops.iter().map(|hop| hop.latency_distribution.presample());
        for (hop, latency) in zip(hops, latencies) {
            let service = self.presampled_services_offset + hop.service;
            let _response = self.children[service]
                .clone()
                .presampled(child::PresampledRequest {
                    latency: Some(latency),
                    sleep: hop.sleep,
                })
                .await?;
        }

        Ok(Response::new(frontend::PresampledResponse {}))
    }
}

impl util::LatencyDistribution {
    fn presample(&self) -> child::Latency {
        let latency = match self {
            util::LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => child::latency::LatencyType::Periodic(Periodic {
                slow_latency: *slow_latency,
                fast_latency: *fast_latency,
                slow_duration_ms: *slow_duration_ms as u32,
            }),
            x => child::latency::LatencyType::Fixed(Fixed {
                latency: x.sample(),
            }),
        };

        child::Latency {
            latency_type: Some(latency),
        }
    }
}
