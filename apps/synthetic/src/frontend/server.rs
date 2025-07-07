pub mod synthetic_tonic {
    pub mod frontend {
        tonic::include_proto!("frontend");
    }

    pub mod child {
        tonic::include_proto!("child");
    }
}

use std::time::Instant;

use crate::config::SyntheticConfig;
use app_utils::timing::time_now;
use ginepro::LoadBalancedChannel;

use tonic::{Request, Response, Status};

use synthetic_tonic::{
    child, child::child_client::ChildClient, frontend, frontend::frontend_server::Frontend,
};

pub struct FrontendImpl {
    child1_client: ChildClient<LoadBalancedChannel>,
    child2_client: ChildClient<LoadBalancedChannel>,
}

impl FrontendImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let channel =
            LoadBalancedChannel::builder((config.child_ips[0].clone(), config.child_ports[0]))
                .channel()
                .await
                .expect("Failed to connect to child");
        let child1_client = ChildClient::new(channel);

        let channel =
            LoadBalancedChannel::builder((config.child_ips[1].clone(), config.child_ports[1]))
                .channel()
                .await
                .expect("Failed to connect to child");
        let child2_client = ChildClient::new(channel);

        FrontendImpl {
            child1_client,
            child2_client,
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
        let mut child1_client = self.child1_client.clone();
        let response = child1_client
            .random_latency(child::RandomLatencyRequest {
                sent_at: time_now(),
            })
            .await?;
        let child1_response = response.into_inner();

        let mut child2_client = self.child2_client.clone();
        let response = child2_client
            .constant_latency(child::ConstantLatencyRequest {
                sent_at: time_now(),
            })
            .await?;
        let child2_response = response.into_inner();

        Ok(Response::new(frontend::AResponse {
            child1_queueing_latency: child1_response.queueing_latency,
            child1_sleep_latency: child1_response.sleep_latency,
            child1_handler_latency: child1_response.handler_latency,
            child2_queueing_latency: child2_response.queueing_latency,
            child2_handler_latency: child2_response.handler_latency,
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }
}
