use std::time::Instant;

use crate::bootstrap::{ConnectionBootstrap, ConnectionBootstrapTask};
use crate::config::{parse_call_sequences, SynthbenchConfig};
use crate::hop_trace::encode_hop_traces;
use crate::service_registry::ServiceRegistry;
#[cfg(not(any(feature = "sched_oracle", feature = "eval_oracle_continuation")))]
use crate::{config::ParsedCall, util::execute_call_sequence};
#[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
use crate::{oracle::OraclePlanner, util::execute_oracle_call_sequence};
#[cfg(not(any(feature = "sched_oracle", feature = "eval_oracle_continuation")))]
use std::collections::HashMap;

use tonic::{Request, Response, Status};

use crate::tonic::{frontend, frontend::frontend_server::Frontend};

pub struct FrontendImpl {
    #[cfg(not(any(feature = "sched_oracle", feature = "eval_oracle_continuation")))]
    parsed_entry_points: HashMap<String, Vec<Vec<ParsedCall>>>,
    #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
    oracle_planner: OraclePlanner,
    service_registry: ServiceRegistry,
    _bootstrap_task: Option<ConnectionBootstrapTask>,
}

impl FrontendImpl {
    pub async fn new(config: SynthbenchConfig) -> Self {
        let mut call_graph = config.call_graph;
        #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
        let oracle_work_estimate = config.oracle_work_estimate;

        // Read project name from environment variable (set by exp_runner.runner)
        let project_name = std::env::var("DOCKER_COMPOSE_PROJECT_NAME")
            .ok()
            .filter(|s| !s.is_empty());

        // Parse and validate call sequences
        if let Err(e) = parse_call_sequences(&mut call_graph) {
            panic!("Failed to parse call graph: {}", e);
        }

        #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
        let oracle_planner =
            OraclePlanner::new(&call_graph, oracle_work_estimate).unwrap_or_else(|e| {
                panic!("Failed to initialize oracle planner: {}", e);
            });

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

        FrontendImpl {
            #[cfg(not(any(feature = "sched_oracle", feature = "eval_oracle_continuation")))]
            parsed_entry_points: call_graph.parsed_entry_points,
            #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
            oracle_planner,
            service_registry: registry,
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
        #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
        let oracle_plan = self
            .oracle_planner
            .plan_entry_point("a")
            .map_err(Status::internal)?;

        #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
        let hop_traces =
            execute_oracle_call_sequence(&self.service_registry, &oracle_plan.child_steps).await?;

        #[cfg(not(any(feature = "sched_oracle", feature = "eval_oracle_continuation")))]
        let hop_traces = if let Some(sequence) = self.parsed_entry_points.get("a") {
            execute_call_sequence(&self.service_registry, sequence).await?
        } else {
            // warn!("No entry point defined for 'a'");
            Vec::new()
        };

        let handler_latency = Instant::now().duration_since(start).as_micros() as u64;
        #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
        self.oracle_planner
            .record_entry_point_latency("a", handler_latency, &oracle_plan);

        Ok(Response::new(frontend::AResponse {
            handler_latency,
            hop_trace_json: encode_hop_traces(&hop_traces)?,
        }))
    }

    async fn handle_b(
        &self,
        _request: Request<frontend::BRequest>,
    ) -> Result<Response<frontend::BResponse>, Status> {
        #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
        let start = Instant::now();

        // Call the entry point sequence for "b"
        #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
        let oracle_plan = self
            .oracle_planner
            .plan_entry_point("b")
            .map_err(Status::internal)?;

        #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
        let hop_traces =
            execute_oracle_call_sequence(&self.service_registry, &oracle_plan.child_steps).await?;

        #[cfg(not(any(feature = "sched_oracle", feature = "eval_oracle_continuation")))]
        let hop_traces = if let Some(sequence) = self.parsed_entry_points.get("b") {
            execute_call_sequence(&self.service_registry, sequence).await?
        } else {
            // warn!("No entry point defined for 'b'");
            Vec::new()
        };

        #[cfg(any(feature = "sched_oracle", feature = "eval_oracle_continuation"))]
        self.oracle_planner.record_entry_point_latency(
            "b",
            Instant::now().duration_since(start).as_micros() as u64,
            &oracle_plan,
        );

        Ok(Response::new(frontend::BResponse {
            hop_trace_json: encode_hop_traces(&hop_traces)?,
        }))
    }
}
