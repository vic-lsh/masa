use std::collections::HashMap;
use std::time::Instant;

use crate::bootstrap::{ConnectionBootstrap, ConnectionBootstrapTask};
use crate::config::{
    parse_call_sequences, CallTarget, EstimationMode, ServiceMethod, SyntheticConfig,
};
use crate::service_registry::ServiceRegistry;
use crate::util::{build_oracle_call_plan, execute_call_sequence, execute_oracle_call_plan};

use tonic::{Request, Response, Status};

use crate::tonic::{frontend, frontend::frontend_server::Frontend};

pub struct FrontendImpl {
    parsed_entry_points: HashMap<String, Vec<Vec<(CallTarget, f64)>>>,
    service_registry: ServiceRegistry,
    method_lookup: HashMap<(String, String), ServiceMethod>,
    estimation_mode: EstimationMode,
    _bootstrap_task: Option<ConnectionBootstrapTask>,
}

impl FrontendImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let estimation_mode = config.estimation_mode;
        let SyntheticConfig { mut call_graph, .. } = config;

        // Read project name from environment variable (set by exp_runner.runner)
        let project_name = std::env::var("DOCKER_COMPOSE_PROJECT_NAME")
            .ok()
            .filter(|s| !s.is_empty());

        // Parse and validate call sequences
        if let Err(e) = parse_call_sequences(&mut call_graph) {
            panic!("Failed to parse call graph: {}", e);
        }

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

        // Use ServiceRegistry
        let registry = ServiceRegistry::new();

        // Spawn bootstrap task to connect asynchronously
        let bootstrap_task = if services_to_connect.is_empty() {
            None
        } else {
            let bootstrap = ConnectionBootstrap::new(services_to_connect, registry.clients());
            Some(bootstrap.spawn())
        };

        let mut method_lookup = HashMap::new();
        for service in &call_graph.services {
            for method in &service.methods {
                method_lookup.insert((service.id.clone(), method.name.clone()), method.clone());
            }
        }

        FrontendImpl {
            parsed_entry_points: call_graph.parsed_entry_points,
            service_registry: registry,
            method_lookup,
            estimation_mode,
            _bootstrap_task: bootstrap_task,
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

        // Call the entry point sequence for "a"
        if let Some(sequence) = self.parsed_entry_points.get("a") {
            match self.estimation_mode {
                EstimationMode::Normal => {
                    execute_call_sequence(&self.service_registry, sequence).await?;
                }
                EstimationMode::PerfectSampled => {
                    let plan = build_oracle_call_plan(sequence, &self.method_lookup, 0)?;
                    execute_oracle_call_plan(&self.service_registry, &plan).await?;
                }
            }
        } else {
            // warn!("No entry point defined for 'a'");
        }

        Ok(Response::new(frontend::AResponse {
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
        }))
    }

    async fn handle_b(
        &self,
        _request: Request<frontend::BRequest>,
    ) -> Result<Response<frontend::BResponse>, Status> {
        // Call the entry point sequence for "b"
        if let Some(sequence) = self.parsed_entry_points.get("b") {
            match self.estimation_mode {
                EstimationMode::Normal => {
                    execute_call_sequence(&self.service_registry, sequence).await?;
                }
                EstimationMode::PerfectSampled => {
                    let plan = build_oracle_call_plan(sequence, &self.method_lookup, 0)?;
                    execute_oracle_call_plan(&self.service_registry, &plan).await?;
                }
            }
        } else {
            // warn!("No entry point defined for 'b'");
        }

        Ok(Response::new(frontend::BResponse {}))
    }
}
