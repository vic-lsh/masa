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
use app_utils::timing::time_now;
use ginepro::LoadBalancedChannel;

use tonic::{Request, Response, Status};

use synthetic_tonic::{
    child, child::child_client::ChildClient, frontend, frontend::frontend_server::Frontend,
};

pub struct FrontendImpl {
    children: Vec<ChildClient<LoadBalancedChannel>>,
    constant_replicas: u8,
    next_constant_replica: AtomicU8,
    presampled_replicas: u8,
    presampled_servers_offset: u8,
    presampled_request_types: HashMap<String, Vec<util::Hop>>,
}

impl FrontendImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let replicas = 1 + config.child_constant_replicas + config.child_presampled_replicas;
        let mut children = Vec::new();
        for i in 1..(replicas + 1) {
            let hostname = format!("local-child-service-{}", i);
            let channel = LoadBalancedChannel::builder((hostname, 8000))
                .channel()
                .await
                .expect(&format!("Failed to connect to child {}", i));
            children.push(ChildClient::new(channel))
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
            presampled_replicas: config.child_presampled_replicas,
            presampled_servers_offset: 1 + config.child_constant_replicas,
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
        let latencies = hops.iter().map(|hop| hop.latency_distribution.sample());
        for (hop, latency) in zip(hops, latencies) {
            let server = self.presampled_servers_offset as usize + hop.server;
            let _response = self.children[server]
                .clone()
                .presampled(child::PresampledRequest {
                    latency,
                    sleep: hop.sleep,
                })
                .await?;
        }

        Ok(Response::new(frontend::PresampledResponse {}))
    }
}
