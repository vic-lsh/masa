use std::collections::HashMap;
use std::iter::zip;
use std::{
    sync::atomic::{AtomicU8, Ordering},
    time::Instant,
};

use app_utils::timing::time_now;
use synthetic::config::SyntheticConfig;
use synthetic::util;

use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::{Request, Response, Status};

use synthetic::tonic::{
    child, child::child_client::ChildClient, child::Fixed, child::Periodic, frontend,
    frontend::frontend_server::Frontend,
};

pub struct FrontendImpl {
    children: Vec<ChildClient<LoadBalancedChannel>>,
    constant_replicas: u8,
    next_constant_replica: AtomicU8,
    presampled_services_offset: usize,
    presampled_request_types: HashMap<String, Vec<util::Hop>>,
}

impl FrontendImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let mut services = vec![1];
        services.extend(vec![config.child_constant_replicas]);
        let presampled_services_offset = services.len();
        services.extend(config.child_presampled_services.iter().map(|v| v[0] as u8));
        let mut children = Vec::new();
        let mut start_id = 1;
        for r in services {
            let hostname_base = "local-child-service";
            children.push(ChildClient::new(
                LoadBalancedChannel::new_from(hostname_base.to_string(), 8000, r, start_id).await,
            ));
            start_id += r;
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
        let mut hop1 = self.children.last().unwrap().clone();
        // let response = child_random_client
        //     .random_latency(child::RandomLatencyRequest {
        //         sent_at: time_now(),
        //     })
        //     .await?;
        // let child_random_response = response.into_inner();

        let response = hop1
            .random_latency(child::RandomLatencyRequest {
                sent_at: time_now(),
                busy_spin: true,
            })
            .await?;
        let hop1_response = response.into_inner();

        // let next =
        //     self.next_constant_replica.fetch_add(1, Ordering::SeqCst) % self.constant_replicas;
        // let mut child_constant_client = self.children[next as usize].clone();

        let mut child_constant_client = self.children.first().unwrap().clone();
        let response = child_constant_client
            .constant_latency(child::ConstantLatencyRequest {
                sent_at: time_now(),
                busy_spin: false,
                duration_us: Some(10000),
            })
            .await?;
        let child_constant_response = response.into_inner();

        Ok(Response::new(frontend::AResponse {
            child1_queueing_latency: 0,
            child1_sleep_latency: 0,
            child1_handler_latency: 0,
            child2_queueing_latency: child_constant_response.queueing_latency,
            child2_handler_latency: child_constant_response.handler_latency,
            child2_reply_latency: time_now() - child_constant_response.finished_at,
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }

    async fn handle_b(
        &self,
        _request: Request<frontend::BRequest>,
    ) -> Result<Response<frontend::BResponse>, Status> {
        let _start = Instant::now();
        // let mut child_random_client = self.children.last().unwrap().clone();
        // let response = child_random_client
        //     .random_latency(child::RandomLatencyRequest {
        //         sent_at: time_now(),
        //     })
        //     .await?;
        // let _child_random_response = response.into_inner();

        let mut hop1 = self.children.last().unwrap().clone();
        let response = hop1
            .random_latency(child::RandomLatencyRequest {
                sent_at: time_now(),
                busy_spin: true,
            })
            .await?;
        let hop1_response = response.into_inner();

        // let next =
        //     self.next_constant_replica.fetch_add(1, Ordering::SeqCst) % self.constant_replicas;
        // let mut child_constant_client = self.children[next as usize].clone();

        let mut child_constant_client = self.children.first().unwrap().clone();
        let response = child_constant_client
            .constant_latency(child::ConstantLatencyRequest {
                sent_at: time_now(),
                busy_spin: false,
                duration_us: Some(100000),
            })
            .await?;
        let _child_constant_response = response.into_inner();

        Ok(Response::new(frontend::BResponse {}))
    }

    async fn handle_presampled(
        &self,
        request: Request<frontend::PresampledRequest>,
    ) -> Result<Response<frontend::PresampledResponse>, Status> {
        let request_type = request.into_inner().request_type;
        let hops = self.presampled_request_types.get(&request_type).unwrap();
        // sample latencies
        let latencies: Vec<child::Latency> = hops
            .iter()
            .map(|hop| hop.latency_distribution.presample())
            .collect();
        let concrete_latencies = latencies.iter().map(child_latency_to_value).collect();
        let remaining_execution_times = reversed_prefix_sum(&concrete_latencies);
        for (hop, (latency, remaining)) in zip(hops, zip(latencies, remaining_execution_times)) {
            let service = self.presampled_services_offset + hop.service;
            let mut request = Request::new({
                child::PresampledRequest {
                    latency: Some(latency),
                    sleep: hop.sleep,
                }
            });
            request
                .metadata_mut()
                .insert("remaining_execution_time", remaining.into());
            let _response = self.children[service].clone().presampled(request).await?;
        }

        Ok(Response::new(frontend::PresampledResponse {}))
    }
}

fn reversed_prefix_sum(v: &Vec<u64>) -> Vec<u64> {
    let mut result = Vec::new();
    result.push(*v.last().expect("array is empty"));

    for x in v.iter().rev().skip(1) {
        result.push(x + *result.last().unwrap());
    }

    result.reverse();
    result
}

fn child_latency_to_value(latency: &child::Latency) -> u64 {
    match latency.latency_type.as_ref().unwrap() {
        child::latency::LatencyType::Periodic(Periodic {
            slow_latency,
            fast_latency,
            slow_duration_ms,
        }) => {
            let slow_fraction = *slow_duration_ms as f64 / 1000.0;
            let average =
                slow_fraction * *slow_latency as f64 + (1.0 - slow_fraction) * *fast_latency as f64;
            average.round() as u64
        }
        child::latency::LatencyType::Fixed(Fixed { latency }) => *latency,
    }
}
