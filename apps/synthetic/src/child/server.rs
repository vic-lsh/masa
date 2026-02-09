use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio;
use tokio::runtime::current_thread_queue_len;
use tokio::task::JoinHandle;
use tonic::{Request, Response, Status};

use crate::bootstrap::{ConnectionBootstrap, ConnectionBootstrapTask};
use crate::config::{parse_call_sequences, ServiceMethod, SyntheticConfig};
use crate::service_registry::ServiceRegistry;
use crate::tonic::{child, child::child_server::Child};
use crate::util::{execute_call_sequence, simulate_work};
use app_utils::timing::time_now;

pub struct ChildImpl {
    _service_id: String,
    service_registry: ServiceRegistry,
    method_lookup: HashMap<(String, String), ServiceMethod>,
    _bootstrap_task: Option<ConnectionBootstrapTask>,
    _queue_monitor_task: QueueMonitorTask,
}

struct QueueMonitorTask {
    handle: JoinHandle<()>,
}

impl QueueMonitorTask {
    fn spawn() -> Self {
        let handle = tokio::spawn(async {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            let start_time = Instant::now();
            loop {
                interval.tick().await;
                let queue_len = current_thread_queue_len();
                let elapsed = start_time.elapsed();
                println!(
                    "current_thread_queue_len: {} (elapsed: {:?})",
                    queue_len, elapsed
                );
            }
        });

        Self { handle }
    }
}

impl Drop for QueueMonitorTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl ChildImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        let queue_monitor_task = QueueMonitorTask::spawn();

        // Handle call graph configuration
        let mut call_graph = config.call_graph;

        // Parse and validate call sequences
        if let Err(e) = parse_call_sequences(&mut call_graph) {
            panic!("Failed to parse call graph: {}", e);
        }

        // Build connection info for service registry
        // Docker Compose creates containers with names like: {project}-{service}-{replica_number}
        // We need to connect to individual replica endpoints: {project}-local-{service-id}-service-1, -2, etc.
        // Read project name from environment variable (set by exp_runner.runner)
        let project_name = std::env::var("DOCKER_COMPOSE_PROJECT_NAME")
            .ok()
            .filter(|s| !s.is_empty());

        let registry = ServiceRegistry::new();
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

        // Spawn bootstrap task to connect asynchronously
        let bootstrap_task = if services_to_connect.is_empty() {
            None
        } else {
            let bootstrap = ConnectionBootstrap::new(services_to_connect, registry.clients());
            Some(bootstrap.spawn())
        };

        // Determine current service ID from environment variable
        // This should be set when deploying the service
        let current_service_id = std::env::var("SERVICE_ID")
            .ok()
            .or_else(|| {
                // Fallback: try to infer from hostname
                // In docker compose, hostname might be like "synthetic-child-service-1"
                // For now, we'll require SERVICE_ID to be set explicitly
                None
            })
            .expect("SERVICE_ID environment variable must be set when using call graph");

        // Pre-compute method lookup table
        let mut method_lookup = HashMap::new();
        for service in &call_graph.services {
            for method in &service.methods {
                method_lookup.insert((service.id.clone(), method.name.clone()), method.clone());
            }
        }

        ChildImpl {
            _service_id: current_service_id,
            service_registry: registry,
            method_lookup,
            _bootstrap_task: bootstrap_task,
            _queue_monitor_task: queue_monitor_task,
        }
    }

    fn get_method(&self, service_id: &str, method_name: &str) -> Result<&ServiceMethod, Status> {
        self.method_lookup
            .get(&(service_id.to_string(), method_name.to_string()))
            .ok_or_else(|| {
                Status::not_found(format!(
                    "Method '{}' not found in service '{}'",
                    method_name, service_id
                ))
            })
    }
}

#[tonic::async_trait]
impl Child for ChildImpl {
    async fn run_synthetic(
        &self,
        _request: Request<child::RunSyntheticRequest>,
    ) -> Result<Response<child::RunSyntheticResponse>, Status> {
        Err(Status::unimplemented("Traditional mode not supported"))
    }

    async fn handle_method(
        &self,
        request: Request<child::MethodRequest>,
    ) -> Result<Response<child::MethodResponse>, Status> {
        let request = request.into_inner();
        let queueing_latency = time_now() - request.sent_at;
        let start = Instant::now();

        // Get the method definition
        let method = self.get_method(&request.service_id, &request.method_name)?;

        // Sample latency from method's distribution
        let duration_us = method.latency_distribution.sample();

        // Execute call sequence
        if !method.parsed_call_sequence.is_empty() {
            execute_call_sequence(&self.service_registry, &method.parsed_call_sequence).await?;
        }

        let busy_spin_ratio = method.busy_spin_ratio.unwrap_or(0.1);

        simulate_work(duration_us, busy_spin_ratio).await;

        Ok(Response::new(child::MethodResponse {
            queueing_latency,
            handler_latency: Instant::now().duration_since(start).as_micros() as u64,
            finished_at: time_now(),
        }))
    }
}
