use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio;
use tokio::runtime::current_thread_queue_len;
use tonic::metadata::MetadataValue;
use tonic::{Request, Response, Status};

use crate::bootstrap::ConnectionBootstrap;
use crate::config::{parse_call_sequences, CallTarget, ServiceMethod, SyntheticConfig};
use crate::service_registry::ServiceRegistry;
use crate::tonic::{child, child::child_server::Child};
use crate::util::{should_make_call, simulate_work};
use app_utils::timing::time_now;
use tonic::masa::{METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER};
use tracing::warn;

pub struct ChildImpl {
    _service_id: String,
    service_registry: ServiceRegistry,
    method_lookup: HashMap<(String, String), ServiceMethod>,
}

impl ChildImpl {
    pub async fn new(config: SyntheticConfig) -> Self {
        // Spawn a task that prints the queue length every 500ms
        tokio::spawn(async {
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
        if !services_to_connect.is_empty() {
            let bootstrap = ConnectionBootstrap::new(services_to_connect, registry.clients());
            bootstrap.spawn();
        }

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
        }
    }

    async fn execute_call_sequence(
        &self,
        call_sequence: &[Vec<(CallTarget, f64)>],
    ) -> Result<(), Status> {
        let registry = &self.service_registry;

        // Execute steps sequentially
        for step in call_sequence {
            // Collect tasks for parallel calls in this step
            let mut tasks = Vec::new();

            for (target, probability) in step {
                if should_make_call(*probability) {
                    let client = registry.get_client_clone(&target.service_id).await;

                    let client = match client {
                        Some(client) => client,
                        None => {
                            warn!(
                                "Service '{}' not yet connected, skipping call",
                                target.service_id
                            );
                            continue;
                        }
                    };

                    let sent_at = time_now();

                    // Spawn task to make the call
                    let target_service_id = target.service_id.clone();
                    let target_method_name = target.method_name.clone();
                    let task = tokio::spawn(async move {
                        // Create metadata values first to avoid cloning strings
                        let method_meta = MetadataValue::try_from(target_method_name.as_str())
                            .map_err(|e| {
                                Status::internal(format!(
                                    "Failed to create method name override: {:?}",
                                    e
                                ))
                            })?;
                        let service_meta = MetadataValue::try_from(target_service_id.as_str())
                            .map_err(|e| {
                                Status::internal(format!(
                                    "Failed to create service name override: {:?}",
                                    e
                                ))
                            })?;

                        let mut request = Request::new(crate::tonic::child::MethodRequest {
                            service_id: target_service_id,
                            method_name: target_method_name,
                            sent_at,
                        });

                        request
                            .metadata_mut()
                            .insert(METHOD_NAME_OVERRIDE_HEADER, method_meta);
                        request
                            .metadata_mut()
                            .insert(SERVICE_NAME_OVERRIDE_HEADER, service_meta);

                        client.clone().handle_method(request).await
                    });

                    tasks.push(task);
                }
            }

            // Wait for all parallel calls in this step to complete
            for task in tasks {
                task.await
                    .map_err(|e| Status::internal(format!("Task join error: {}", e)))??;
            }
        }

        Ok(())
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
            self.execute_call_sequence(&method.parsed_call_sequence)
                .await?;
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
