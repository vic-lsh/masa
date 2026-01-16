use std::collections::HashMap;
use std::iter::zip;
use std::time::Instant;

use app_utils::timing::time_now;
use rand::Rng;
use synthetic::config::{ChildService, RequestHop, SyntheticConfig};
use synthetic::util;

use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::{Request, Response, Status};

use synthetic::tonic::{
    child, child::child_client::ChildClient, child::Fixed, child::Periodic, frontend,
    frontend::frontend_server::Frontend,
};

pub struct FrontendImpl {
    children: Vec<ChildClient<LoadBalancedChannel>>,
    presampled_services_offset: usize,
    presampled_request_types: HashMap<String, Vec<util::Hop>>,
    service_map: HashMap<String, usize>,
    request_a_hops: Vec<RequestHop>,
    request_b_hops: Vec<RequestHop>,
}

impl FrontendImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let SyntheticConfig {
            child_presampled_services,
            child_presampled_request_types,
            child_services,
            request_a_hops,
            request_b_hops,
            ..
        } = config;

        let mut services = Vec::new();
        let mut random_services = child_services;
        if random_services.is_empty() {
            random_services.push(ChildService {
                id: "default".to_string(),
                replicas: 1,
            });
        }
        services.extend(random_services.iter().map(|svc| svc.replicas));
        let presampled_services_offset = services.len();
        services.extend(child_presampled_services.iter().map(|v| v[0] as u8));
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
        for (key, value) in child_presampled_request_types {
            presampled_request_types.insert(
                key,
                value.into_iter().map(|hop| util::Hop::from(hop)).collect(),
            );
        }

        let mut service_map = HashMap::new();
        for (index, service) in random_services.iter().enumerate() {
            let inserted = service_map.insert(service.id.clone(), index);
            assert!(
                inserted.is_none(),
                "duplicate child service id: {}",
                service.id
            );
        }

        FrontendImpl {
            children,
            presampled_request_types,
            presampled_services_offset,
            service_map,
            request_a_hops,
            request_b_hops,
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
        let results = self.execute_request_hops(&self.request_a_hops).await?;
        if results.len() < 2 {
            return Err(Status::failed_precondition(
                "request_a_hops must contain at least 2 hops",
            ));
        }
        let hop1 = &results[0];
        let hop2 = &results[1];

        Ok(Response::new(frontend::AResponse {
            child1_queueing_latency: hop1.response.queueing_latency,
            child1_sleep_latency: hop1.response.sleep_latency,
            child1_handler_latency: hop1.response.handler_latency,
            child2_queueing_latency: hop2.response.queueing_latency,
            child2_handler_latency: hop2.response.handler_latency,
            child2_reply_latency: hop2.finished_at - hop2.sent_at,
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }

    async fn handle_b(
        &self,
        _request: Request<frontend::BRequest>,
    ) -> Result<Response<frontend::BResponse>, Status> {
        let _start = Instant::now();
        let results = self.execute_request_hops(&self.request_b_hops).await?;
        if results.len() < 2 {
            return Err(Status::failed_precondition(
                "request_b_hops must contain at least 2 hops",
            ));
        }

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

impl FrontendImpl {
    async fn execute_request_hops(&self, hops: &[RequestHop]) -> Result<Vec<HopResult>, Status> {
        if hops.is_empty() {
            return Err(Status::failed_precondition(
                "request hops are not configured",
            ));
        }

        let mut results = Vec::with_capacity(hops.len());
        for hop in hops {
            let service_index = *self.service_map.get(&hop.service_id).ok_or_else(|| {
                Status::invalid_argument(format!("unknown child service id: {}", hop.service_id))
            })?;

            if !(0.0..=1.0).contains(&hop.busy_spin_prob) {
                return Err(Status::invalid_argument(
                    "busy_spin_prob must be between 0.0 and 1.0",
                ));
            }

            let busy_spin = rand::thread_rng().gen_bool(hop.busy_spin_prob);
            let sent_at = time_now();
            let response = self.children[service_index]
                .clone()
                .random_latency(child::RandomLatencyRequest {
                    sent_at,
                    busy_spin,
                    duration_us: hop.duration_us,
                })
                .await?;
            let finished_at = time_now();
            results.push(HopResult {
                response: response.into_inner(),
                sent_at,
                finished_at,
            });
        }

        Ok(results)
    }
}

struct HopResult {
    response: child::RandomLatencyResponse,
    sent_at: u64,
    finished_at: u64,
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
