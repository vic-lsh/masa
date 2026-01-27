use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::bootstrap::ConnectionBootstrap;
use crate::config::{resolve_call_graphs, SyntheticConfig};
use app_utils::timing::time_now;
use tracing::{info, warn};

use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::{Request, Response, Status};

use crate::tonic::{
    child, child::child_client::ChildClient, frontend, frontend::frontend_server::Frontend,
};

pub struct FrontendImpl {
    api_entry_points: HashMap<String, (String, String)>, // api -> (service_id, method_name)
    call_graph_clients: Arc<RwLock<HashMap<String, ChildClient<LoadBalancedChannel>>>>,
}

impl FrontendImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let resolved = resolve_call_graphs(&config)
            .unwrap_or_else(|e| panic!("Failed to resolve call graphs: {}", e));
        info!(
            "Resolved call graph services: {:?}",
            resolved
                .services
                .iter()
                .map(|svc| svc.id.as_str())
                .collect::<Vec<_>>()
        );

        // Read project name from environment variable (set by exp.runner)
        let project_name = std::env::var("DOCKER_COMPOSE_PROJECT_NAME")
            .ok()
            .filter(|s| !s.is_empty());

        // Handle call graph configuration
        // Build connection info for all services in merged call graph
        // Docker Compose creates containers with names like: {project}-{service}-{replica_number}
        // We need to connect to individual replica endpoints: {project}-local-{service-id}-service-1, -2, etc.
        let mut services_to_connect = Vec::new();
        for service in &resolved.services {
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

        let api_entry_points = resolved
            .api_entry_points
            .into_iter()
            .map(|(api, target)| (api, (target.service_id, target.method_name)))
            .collect();

        FrontendImpl {
            api_entry_points,
            call_graph_clients: clients,
        }
    }

    async fn call_entry_point(&self, api: &str) -> Result<(), Status> {
        let (service_id, method_name) = self.api_entry_points.get(api).ok_or_else(|| {
            Status::invalid_argument(format!("Unknown API '{}' for call graph", api))
        })?;

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
        request: Request<frontend::ARequest>,
    ) -> Result<Response<frontend::AResponse>, Status> {
        let start = Instant::now();
        let ctx = request
            .metadata()
            .get_ctx("ctx")
            .ok_or_else(|| Status::invalid_argument("Missing ctx metadata"))?;
        let api = ctx.api().to_string();

        self.call_entry_point(&api).await?;

        Ok(Response::new(frontend::AResponse {
            child1_queueing_latency: 0,
            child1_sleep_latency: 0,
            child1_handler_latency: 0,
            child2_queueing_latency: 0,
            child2_handler_latency: 0,
            child2_reply_latency: 0,
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }

    async fn handle_b(
        &self,
        request: Request<frontend::BRequest>,
    ) -> Result<Response<frontend::BResponse>, Status> {
        let ctx = request
            .metadata()
            .get_ctx("ctx")
            .ok_or_else(|| Status::invalid_argument("Missing ctx metadata"))?;
        let api = ctx.api().to_string();
        self.call_entry_point(&api).await?;
        Ok(Response::new(frontend::BResponse {}))
    }
}
