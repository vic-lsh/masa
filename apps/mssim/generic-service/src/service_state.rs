use crate::bootstrap::ConnectionBootstrap;
use crate::busy_spin;
use crate::parent_chain::{encode_parent_chain, PARENT_CHAIN_METADATA_KEY};
use crate::service_replay::ReplaySpanExecutor;
use crate::service_stubs::{InvokeRequest, ReplayRequest};
use crate::RpcClient;
use anyhow::{Context, Result};
use sim_config::deployment::Deployment;
use sim_config::svc::call_sequence::{
    load_call_sequence, load_root_user_call_sequence, CallSequence,
};
use sim_config::svc::{MethodId, ServiceName, ServiceTraceConfig};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard};
use tonic::{Request, Status};
use tracing::{error, warn};

pub(crate) struct ServiceState {
    config: ServiceTraceConfig,
    pub(crate) clients: Arc<RwLock<HashMap<ServiceName, RpcClient>>>,
    self_svc_name: ServiceName,
    overshot_counter: AtomicUsize,
    call_sequence: Option<CallSequence>,
    // USER call sequence loaded at startup (one per service instance)
    user_call_sequence: CallSequence,
}

impl ServiceState {
    pub(crate) fn initialize(
        self_svc_name: ServiceName,
        config: ServiceTraceConfig,
        deployment: Deployment,
        config_dir: PathBuf,
    ) -> Result<(Arc<Self>, Option<ConnectionBootstrap>)> {
        let child_weights = config.call_graph.callees_of(&self_svc_name);
        let clients = Arc::new(RwLock::new(HashMap::new()));

        println!("Child services:");
        for child in child_weights.keys() {
            println!("{}", child.as_str());
        }

        // Load call sequence if available
        let call_sequence = load_call_sequence(&config_dir, &self_svc_name)
            .context("Failed to load call sequence")?;

        // Load USER call sequence at startup for root API
        let user_call_sequence = load_root_user_call_sequence(&config_dir)
            .context("Failed to load USER call sequence")?;

        println!("USER call sequence: {:?}", user_call_sequence);

        let bootstrap = if child_weights.is_empty() {
            None
        } else {
            let children: Vec<_> = child_weights.keys().cloned().collect();
            let children_for_log: Vec<_> = child_weights
                .iter()
                .map(|(svc, weight)| (svc.clone(), *weight))
                .collect();
            Some(ConnectionBootstrap::new(
                children,
                children_for_log,
                deployment,
                Arc::clone(&clients),
            ))
        };

        let state = Arc::new(ServiceState {
            config,
            clients,
            self_svc_name,
            overshot_counter: AtomicUsize::new(0),
            call_sequence,
            user_call_sequence,
        });

        Ok((state, bootstrap))
    }

    pub(crate) fn self_service_name(&self) -> &ServiceName {
        &self.self_svc_name
    }

    pub(crate) async fn handle_method(
        &self,
        method_name: String,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: Option<&str>,
    ) -> Result<(), Status> {
        let graph_selection = graph_name.unwrap().trim();
        let graph_ref = Some(graph_selection);

        let method_latency = self.config.method_latency.as_ref().ok_or_else(|| {
            Status::internal("Configuration error: method latency not configured")
        })?;
        let method_id: MethodId = method_name.clone().into();
        let latency_dist = method_latency
            .get_method_dist(&method_id, graph_ref)
            .ok_or_else(|| Status::not_found("Method not found"))?;

        let total_latency_ms = latency_dist.sample(&mut rand::rng());

        let start_time = std::time::Instant::now();
        self.fanout(req_id, start_at, parent_chain, graph_ref)
            .await?;
        let elapsed = start_time.elapsed();

        let remaining = total_latency_ms - (elapsed.as_millis() as f64);
        if remaining > 0.0 {
            busy_spin(std::time::Duration::from_millis(remaining as u64));
        }

        Ok(())
    }

    pub(crate) async fn fanout(
        &self,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: Option<&str>,
    ) -> Result<(), Status> {
        let graph_selection = graph_name.unwrap().trim();

        // Call sequence is required - panic if not available
        let call_sequence = self.call_sequence.as_ref().expect(&format!(
            "Call sequence is required for service {} but was not found",
            self.self_svc_name.as_str()
        ));

        self.fanout_with_call_sequence(
            req_id,
            start_at,
            parent_chain,
            graph_selection,
            call_sequence,
        )
        .await
    }

    async fn fanout_with_call_sequence(
        &self,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: &str,
        call_sequence: &CallSequence,
    ) -> Result<(), Status> {
        let mut parent_chain_for_children = parent_chain.clone();
        parent_chain_for_children.push(self.self_svc_name.clone());
        let parent_chain_metadata = encode_parent_chain(&parent_chain_for_children)?;

        let clients_guard = self.clients.read().await;

        // Execute each step sequentially
        for step in call_sequence {
            let mut tasks = Vec::new();

            // Process each child in this step
            for entry in step {
                let child_svc_name = &entry.service_name;

                // Skip if this is a self-call or creates a cycle
                if child_svc_name == &self.self_svc_name {
                    continue;
                }

                if parent_chain.iter().any(|svc| svc == child_svc_name) {
                    continue;
                }

                // Check probability
                if entry.probability <= 0.0 {
                    continue;
                }

                if entry.probability < 1.0 && rand::random::<f64>() >= entry.probability {
                    continue;
                }

                // Get client for this child service
                let client = match clients_guard.get(child_svc_name) {
                    Some(client) => client.clone(),
                    None => {
                        warn!(
                            "Child service {} not found in clients map",
                            child_svc_name.as_str()
                        );
                        continue;
                    }
                };

                // Create request
                let mut client_clone = client.clone();
                let mut request = Request::new(InvokeRequest {
                    req_id,
                    start_at,
                    method_name: entry.method_name.clone(),
                    graph_name: graph_name.to_string(),
                });

                if let Some(ref metadata_value) = parent_chain_metadata {
                    request
                        .metadata_mut()
                        .insert(PARENT_CHAIN_METADATA_KEY, metadata_value.clone());
                }

                // Spawn task for this child
                let child = child_svc_name.clone();
                let handle = tokio::spawn(async move {
                    client_clone.invoke(request).await.map_err(|e| {
                        Status::internal(format!("RPC to child service failed: {:?}", e))
                    })
                });
                tasks.push((child, handle));
            }

            // Wait for all tasks in this step to complete before proceeding to next step
            for (child_svc, handle) in tasks {
                let rpc_result = handle
                    .await
                    .map_err(|e| Status::internal(format!("Task join error: {:?}", e)))?;
                rpc_result.map_err(|err| {
                    error!(
                        "RPC to child service {} failed: {:?}",
                        child_svc.as_str(),
                        err
                    );
                    err
                })?;
            }
        }

        Ok(())
    }

    fn sample_method_for_child(
        &self,
        child_svc_name: &ServiceName,
        graph_name: &str,
    ) -> Option<(String, Option<String>)> {
        if let Some(freq_map) = self.config.method_freq_map.as_ref() {
            let mut rng = rand::rng();
            if let Some(sampled) = freq_map.sample_method(child_svc_name, graph_name, &mut rng) {
                return Some((sampled.method, sampled.graph));
            }
        }

        None
    }

    pub(crate) async fn execute_replay(&self, request: &ReplayRequest) -> Result<(), Status> {
        let executor = ReplaySpanExecutor::new(self, request).await?;
        executor.run().await
    }

    pub(crate) fn evaluate_replay_timing(
        &self,
        elapsed: std::time::Duration,
        request: &ReplayRequest,
    ) -> Result<(), Status> {
        if elapsed.as_millis() > request.exclude_queue_latency as u128 {
            let previous = self.overshot_counter.fetch_add(1, Ordering::Relaxed);
            if previous % 10 == 0 {
                warn!(
                    "Warning {}: processing took longer ({:?}) than request latency ({} us)",
                    previous, elapsed, request.exclude_queue_latency
                );
            }
        } else if elapsed.as_micros() > request.slo as u128 {
            return Err(Status::cancelled("Processing took more than SLO"));
        }
        Ok(())
    }

    pub(crate) async fn read_clients(
        &self,
    ) -> RwLockReadGuard<'_, HashMap<ServiceName, RpcClient>> {
        self.clients.read().await
    }

    /// Use pre-loaded USER call sequence for root API.
    /// The call sequence is loaded once at startup.
    pub(crate) async fn fanout_with_user_call_sequence(
        &self,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: &str,
    ) -> Result<(), Status> {
        // Use the pre-loaded USER call sequence
        self.fanout_with_call_sequence(
            req_id,
            start_at,
            parent_chain,
            graph_name,
            &self.user_call_sequence,
        )
        .await
    }
}

fn compute_child_probabilities(
    child_weights: &HashMap<ServiceName, u64>,
) -> HashMap<ServiceName, f64> {
    if child_weights.is_empty() {
        return HashMap::new();
    }

    let total_weight = child_weights
        .values()
        .copied()
        .max()
        .expect("Max weight should be available") as f64;

    child_weights
        .iter()
        .map(|(svc, &weight)| {
            let probability = if total_weight > 0.0 {
                (weight as f64) / total_weight
            } else {
                0.0
            };
            (svc.clone(), probability)
        })
        .collect()
}
