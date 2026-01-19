use std::collections::HashMap;
use std::iter::zip;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::bootstrap::ConnectionBootstrap;
use crate::config::{
    parse_call_sequences, parse_service_method, ChildService, LatencyDistribution, RequestHop,
    SyntheticConfig,
};
use crate::util;
use app_utils::timing::time_now;
use tracing::{info, warn};

use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::{Request, Response, Status};

use crate::tonic::{
    child, child::child_client::ChildClient, child::Fixed, child::Periodic, frontend,
    frontend::frontend_server::Frontend,
};

pub struct FrontendImpl {
    children: Vec<ChildClient<LoadBalancedChannel>>,
    presampled_services_offset: usize,
    presampled_request_types: HashMap<String, Vec<util::Hop>>,
    service_map: HashMap<String, usize>,
    random_latency: LatencyDistribution,
    request_a_hops: Vec<RequestHop>,
    request_b_hops: Vec<RequestHop>,
    call_graph_entry_point: Option<(String, String)>, // (service_id, method_name)
    call_graph_clients: Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>>,
}

impl FrontendImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let SyntheticConfig {
            child_random_latency,
            child_presampled_services,
            child_presampled_request_types,
            child_services,
            request_a_hops,
            request_b_hops,
            call_graph,
            ..
        } = config;

        info!("Child random latency: {:?}", child_random_latency);
        info!("Child presampled services: {:?}", child_presampled_services);
        info!(
            "Child presampled request types: {:?}",
            child_presampled_request_types
        );
        info!("Child services: {:?}", child_services);
        info!("Request a hops: {:?}", request_a_hops);
        info!("Request b hops: {:?}", request_b_hops);

        let random_latency = child_random_latency;

        // Handle call graph configuration
        let (
            call_graph_entry_point,
            call_graph_clients,
            children,
            presampled_services_offset,
            presampled_request_types,
            service_map,
        ) = if let Some(mut call_graph) = call_graph {
            // Parse and validate call sequences
            if let Err(e) = parse_call_sequences(&mut call_graph) {
                panic!("Failed to parse call graph: {}", e);
            }

            // Parse entry point
            let entry_target =
                parse_service_method(&call_graph.entry_point).expect("Failed to parse entry point");
            let entry_point = Some((
                entry_target.service_id.clone(),
                entry_target.method_name.clone(),
            ));

            // Build connection info for all services in call graph
            // Docker Compose creates containers with names like: {project}-{service}-{replica_number}
            // We need to connect to individual replica endpoints: {project}-local-{service-id}-service-1, -2, etc.
            // Read project name from environment variable (set by exp.runner)
            let project_name = std::env::var("DOCKER_COMPOSE_PROJECT_NAME")
                .ok()
                .filter(|s| !s.is_empty());

            let mut services_to_connect = Vec::new();
            for service in &call_graph.services {
                // Service name matches the compose file service name: "local-{service-id}-service"
                // Docker Compose creates containers like: {project}-local-{service-id}-service-1, -2, etc.
                let base_service_name = format!("local-{}-service", service.id.to_lowercase());
                let hostname_base = if let Some(ref project) = project_name {
                    format!("{}-{}", project, base_service_name)
                } else {
                    base_service_name
                };
                services_to_connect.push((service.id.clone(), hostname_base, service.replicas));
            }

            // Create empty clients map - will be populated by bootstrap task
            let clients = Arc::new(RwLock::new(HashMap::new()));

            // Spawn bootstrap task to connect asynchronously
            if !services_to_connect.is_empty() {
                let bootstrap = ConnectionBootstrap::new(services_to_connect, Arc::clone(&clients));
                bootstrap.spawn();
            }

            // When using call graph, don't create old-style children clients
            (
                entry_point,
                clients, // Arc<RwLock<HashMap>>
                Vec::new(),
                0,
                HashMap::new(),
                HashMap::new(),
            )
        } else {
            // Traditional mode: create children clients
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
                    LoadBalancedChannel::new_from(hostname_base.to_string(), 8000, r, start_id)
                        .await,
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

            (
                None,
                Arc::new(RwLock::new(HashMap::new())),
                children,
                presampled_services_offset,
                presampled_request_types,
                service_map,
            )
        };

        FrontendImpl {
            children,
            presampled_request_types,
            presampled_services_offset,
            service_map,
            random_latency,
            request_a_hops,
            request_b_hops,
            call_graph_entry_point,
            call_graph_clients,
        }
    }

    async fn call_entry_point(&self) -> Result<(), Status> {
        let (service_id, method_name) = self
            .call_graph_entry_point
            .as_ref()
            .ok_or_else(|| Status::failed_precondition("Call graph not configured"))?;

        let client = self
            .call_graph_clients
            .read()
            .await
            .get(service_id)
            .cloned();

        let client = match client {
            Some(client) => client,
            None => {
                warn!("Service '{}' not yet connected, skipping call", service_id);
                return Err(Status::unavailable(format!(
                    "Service '{}' not yet connected",
                    service_id
                )));
            }
        };

        let sent_at = time_now();
        client
            .clone()
            .handle_method(child::MethodRequest {
                service_id: service_id.clone(),
                method_name: method_name.clone(),
                sent_at,
            })
            .await?;

        Ok(())
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

        // If call graph is configured, use it; otherwise use traditional hops
        if let Some(_) = &self.call_graph_entry_point {
            // Call the entry point method
            self.call_entry_point().await?;

            // For call graph mode, return minimal response
            // (could be enhanced to return more detailed metrics)
            Ok(Response::new(frontend::AResponse {
                child1_queueing_latency: 0,
                child1_sleep_latency: 0,
                child1_handler_latency: 0,
                child2_queueing_latency: 0,
                child2_handler_latency: 0,
                child2_reply_latency: 0,
                handler_latency: Instant::now().duration_since(start).as_micros() as u64,
            }))
        } else {
            // Traditional mode
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
                child1_sleep_latency: hop1.sleep_latency_us(),
                child1_handler_latency: hop1.response.handler_latency,
                child2_queueing_latency: hop2.response.queueing_latency,
                child2_handler_latency: hop2.response.handler_latency,
                child2_reply_latency: time_now() - hop2.response.finished_at,
                handler_latency: Instant::now().duration_since(start).as_micros() as u64,
            }))
        }
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

            let duration_us = hop
                .duration_us
                .unwrap_or_else(|| self.random_latency.sample());
            let busy_spin_dur_us = hop.busy_spin_dur_us.unwrap_or(0);
            if busy_spin_dur_us > duration_us {
                return Err(Status::invalid_argument(format!(
                    "Frontend: busy_spin_dur_us ({}) cannot be greater than duration_us ({})",
                    busy_spin_dur_us, duration_us
                )));
            }
            let sent_at = time_now();
            let response = self.children[service_index]
                .clone()
                .run_synthetic(child::RunSyntheticRequest {
                    sent_at,
                    duration_us: Some(duration_us),
                    busy_spin_dur_us: Some(busy_spin_dur_us),
                })
                .await?;
            results.push(HopResult {
                response: response.into_inner(),
                duration_us,
                busy_spin_dur_us,
            });
        }

        Ok(results)
    }
}

struct HopResult {
    response: child::RunSyntheticResponse,
    duration_us: u64,
    busy_spin_dur_us: u64,
}

impl HopResult {
    fn sleep_latency_us(&self) -> u64 {
        self.duration_us.saturating_sub(self.busy_spin_dur_us)
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
