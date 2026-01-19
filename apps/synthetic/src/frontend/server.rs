use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::bootstrap::ConnectionBootstrap;
use crate::config::{
    parse_call_sequences, parse_service_method, ChildService, RequestHop, SyntheticConfig,
};
use app_utils::timing::time_now;
use tracing::{info, warn};

use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::{Request, Response, Status};

use crate::tonic::{
    child, child::child_client::ChildClient, frontend, frontend::frontend_server::Frontend,
};

pub struct FrontendImpl {
    children: Vec<ChildClient<LoadBalancedChannel>>,
    service_map: HashMap<String, usize>,
    request_a_hops: Vec<RequestHop>,
    request_b_hops: Vec<RequestHop>,
    call_graph_entry_point: Option<(String, String)>, // (service_id, method_name)
    call_graph_clients: Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>>,
}

impl FrontendImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let SyntheticConfig {
            child_services,
            request_a_hops,
            request_b_hops,
            call_graph,
            ..
        } = config;

        info!("Child services: {:?}", child_services);
        info!("Request a hops: {:?}", request_a_hops);
        info!("Request b hops: {:?}", request_b_hops);

        // Handle call graph configuration
        let (call_graph_entry_point, call_graph_clients, children, service_map) =
            if let Some(mut call_graph) = call_graph {
                // Parse and validate call sequences
                if let Err(e) = parse_call_sequences(&mut call_graph) {
                    panic!("Failed to parse call graph: {}", e);
                }

                // Parse entry point
                let entry_target = parse_service_method(&call_graph.entry_point)
                    .expect("Failed to parse entry point");
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
                    let bootstrap =
                        ConnectionBootstrap::new(services_to_connect, Arc::clone(&clients));
                    bootstrap.spawn();
                }

                // When using call graph, don't create old-style children clients
                (
                    entry_point,
                    clients, // Arc<RwLock<HashMap>>
                    Vec::new(),
                    HashMap::new(),
                )
            } else {
                // Traditional mode: create children clients
                let mut random_services = child_services;
                if random_services.is_empty() {
                    random_services.push(ChildService {
                        id: "default".to_string(),
                        replicas: 1,
                    });
                }
                let mut children = Vec::new();
                let mut start_id = 1;
                for svc in &random_services {
                    let hostname_base = "local-child-service";
                    children.push(ChildClient::new(
                        LoadBalancedChannel::new_from(
                            hostname_base.to_string(),
                            8000,
                            svc.replicas,
                            start_id,
                        )
                        .await,
                    ));
                    start_id += svc.replicas;
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
                    service_map,
                )
            };

        FrontendImpl {
            children,
            service_map,
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

            let duration_us = hop.duration_us.ok_or_else(|| {
                Status::invalid_argument("duration_us must be specified for request hop")
            })?;
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
