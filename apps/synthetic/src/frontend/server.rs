use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::bootstrap::ConnectionBootstrap;
use crate::config::{parse_call_sequences, parse_service_method, SyntheticConfig};
use app_utils::timing::time_now;
use tracing::warn;

use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::{Request, Response, Status};

use crate::tonic::{
    child, child::child_client::ChildClient, frontend, frontend::frontend_server::Frontend,
};

pub struct FrontendImpl {
    call_graph_entry_point: (String, String), // (service_id, method_name)
    call_graph_clients: Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>>,
}

impl FrontendImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let SyntheticConfig { mut call_graph, .. } = config;

        // Read project name from environment variable (set by exp_runner.runner)
        let project_name = std::env::var("DOCKER_COMPOSE_PROJECT_NAME")
            .ok()
            .filter(|s| !s.is_empty());

        // Parse and validate call sequences
        if let Err(e) = parse_call_sequences(&mut call_graph) {
            panic!("Failed to parse call graph: {}", e);
        }

        // Parse entry point
        let entry_target =
            parse_service_method(&call_graph.entry_point).expect("Failed to parse entry point");
        let entry_point = (
            entry_target.service_id.clone(),
            entry_target.method_name.clone(),
        );

        // Build connection info for all services in call graph
        // Docker Compose creates containers with names like: {project}-{service}-{replica_number}
        // We need to connect to individual replica endpoints: {project}-local-{service-id}-service-1, -2, etc.

        let mut services_to_connect = Vec::new();
        for service in &call_graph.services {
            // Service name matches the compose file service name: "local-{service-id}-service"
            // Docker Compose creates containers like: {project}-local-{service-id}-service-1, -2, etc.
            let base_service_name = format!(
                "local-{}-service",
                service.id.to_lowercase().replace("_", "-")
            );
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

        FrontendImpl {
            call_graph_entry_point: entry_point,
            call_graph_clients: clients,
        }
    }

    async fn call_entry_point(&self) -> Result<(), Status> {
        let (service_id, method_name) = &self.call_graph_entry_point;

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

        // Call the entry point method
        self.call_entry_point().await?;

        Ok(Response::new(frontend::AResponse {
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }

    async fn handle_b(
        &self,
        _request: Request<frontend::BRequest>,
    ) -> Result<Response<frontend::BResponse>, Status> {
        // Call the entry point method
        self.call_entry_point().await?;

        Ok(Response::new(frontend::BResponse {}))
    }
}
