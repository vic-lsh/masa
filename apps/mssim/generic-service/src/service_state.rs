use crate::bootstrap::ConnectionBootstrap;
use crate::busy_spin;
use crate::parent_chain::{encode_parent_chain, PARENT_CHAIN_METADATA_KEY};
use crate::service_replay::ReplaySpanExecutor;
use crate::service_stubs::{InvokeRequest, ReplayRequest};
use crate::RpcClient;
use anyhow::{Context, Result};
use masa::MethodId;
use sim_config::deployment::Deployment;
use sim_config::svc::call_sequence::{
    get_all_graph_ids, load_call_sequence, load_root_user_call_sequence, CallSequence,
};
use sim_config::svc::{ServiceName, ServiceTraceConfig};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use tokio::sync::{RwLock, RwLockReadGuard};
use tonic::{masa::context::MasaRequestExt, Request, Status};
use tracing::{error, info, warn};

pub(crate) struct ServiceState {
    config: ServiceTraceConfig,
    pub(crate) clients: Arc<RwLock<HashMap<ServiceName, RpcClient>>>,
    self_svc_name: ServiceName,
    overshot_counter: AtomicUsize,
    // Call sequences keyed by graph_id, loaded upfront for all graphs
    call_sequences: HashMap<String, Option<CallSequence>>,
    child_call_probabilities: HashMap<ServiceName, f64>,
    // USER call sequences keyed by graph_id, loaded upfront for all graphs
    user_call_sequences: HashMap<String, CallSequence>,
}

impl ServiceState {
    pub(crate) fn initialize(
        self_svc_name: ServiceName,
        config: ServiceTraceConfig,
        deployment: Deployment,
        callgraph_dirs: Vec<PathBuf>,
    ) -> Result<(Arc<Self>, Option<ConnectionBootstrap>)> {
        info!("Initializing service state for {}", self_svc_name.as_str());

        let child_weights = config.call_graph.callees_of(&self_svc_name);
        let child_call_probabilities = compute_child_probabilities(&child_weights);
        let clients = Arc::new(RwLock::new(HashMap::new()));

        println!("Child services:");
        for child in child_weights.keys() {
            println!("{}", child.as_str());
        }

        // Load call sequences from all call graph directories
        let mut call_sequences: HashMap<String, Option<CallSequence>> = HashMap::new();
        let mut user_call_sequences: HashMap<String, CallSequence> = HashMap::new();

        for callgraph_dir in &callgraph_dirs {
            // Extract graph_id from directory name (e.g., "S_14677443" from "/app/callgraphs/S_14677443")
            let graph_id = callgraph_dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "default".to_string());

            // Try loading call sequence for this graph
            if let Ok(Some(call_sequence)) =
                load_call_sequence(callgraph_dir, &self_svc_name, Some(&graph_id))
            {
                call_sequences.insert(graph_id.clone(), Some(call_sequence));
            } else if let Ok(Some(call_sequence)) =
                load_call_sequence(callgraph_dir, &self_svc_name, None)
            {
                // Fallback: try without graph_id (backward compatibility)
                call_sequences.insert(graph_id.clone(), Some(call_sequence));
            }

            // Try loading USER call sequence for this graph
            if let Ok(user_call_sequence) =
                load_root_user_call_sequence(callgraph_dir, Some(&graph_id))
            {
                user_call_sequences.insert(graph_id.clone(), user_call_sequence);
            } else if let Ok(user_call_sequence) = load_root_user_call_sequence(callgraph_dir, None)
            {
                // Fallback: try without graph_id
                user_call_sequences.insert(graph_id.clone(), user_call_sequence);
            }
        }

        // If no call sequences were loaded, try loading from first directory with default behavior
        if call_sequences.is_empty() && !callgraph_dirs.is_empty() {
            let first_dir = &callgraph_dirs[0];
            if let Ok(Some(call_sequence)) = load_call_sequence(first_dir, &self_svc_name, None) {
                call_sequences.insert("default".to_string(), Some(call_sequence));
            }
            if let Ok(user_call_sequence) = load_root_user_call_sequence(first_dir, None) {
                user_call_sequences.insert("default".to_string(), user_call_sequence);
            }
        }

        println!("Loaded call sequences for {} graphs", call_sequences.len());
        println!(
            "USER call sequences: {:?}",
            user_call_sequences.keys().collect::<Vec<_>>()
        );

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
            call_sequences,
            child_call_probabilities,
            user_call_sequences,
        });

        Ok((state, bootstrap))
    }

    pub(crate) fn self_service_name(&self) -> &ServiceName {
        &self.self_svc_name
    }

    pub(crate) async fn handle_method(
        &self,
        method_id: MethodId,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: &str,
    ) -> Result<(), Status> {
        let method_latency = self.config.method_latency.as_ref().ok_or_else(|| {
            Status::internal("Configuration error: method latency not configured")
        })?;
        let latency_dist = method_latency
            .get_method_dist(&method_id, graph_name)
            .ok_or_else(|| Status::not_found("Method not found"))?;

        let total_latency_ms = latency_dist.sample(&mut rand::rng());

        // If this is a leaf service (no child services), directly busy spin
        if self.child_call_probabilities.is_empty() {
            self.handle_leaf_service(total_latency_ms).await;
        } else {
            let start_time = std::time::Instant::now();
            self.fanout(req_id, start_at, parent_chain, graph_name)
                .await?;
            let elapsed = start_time.elapsed();

            let remaining = total_latency_ms - (elapsed.as_millis() as f64);
            busy_spin(std::time::Duration::from_millis(remaining.max(1.0) as u64));
        }

        Ok(())
    }

    async fn handle_leaf_service(&self, total_latency_ms: f64) {
        const SPIN_FRACTION: f64 = 0.2;

        let spin_duration = total_latency_ms * SPIN_FRACTION;
        let block_duration = total_latency_ms - spin_duration;

        busy_spin(std::time::Duration::from_millis(spin_duration as u64));
        tokio::time::sleep(std::time::Duration::from_millis(block_duration as u64)).await;
    }

    pub(crate) async fn fanout(
        &self,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: &str,
    ) -> Result<(), Status> {
        static CALL_SEQUENCE_MISSING_WARN_ONCE: Once = Once::new();

        // Look up call sequence for this graph
        // graph_name might be in format "s-14677443" but graph_id in file is "S_14677443"
        // Try both formats
        let graph_id_variants = vec![
            graph_name.to_string(),
            graph_name.replace("s-", "S_"),
            graph_name.replace("-", "_"),
        ];

        let mut call_sequence_opt = None;
        for variant in &graph_id_variants {
            if let Some(seq) = self.call_sequences.get(variant) {
                call_sequence_opt = seq.as_ref();
                break;
            }
        }

        // If not found, try "default" as fallback
        if call_sequence_opt.is_none() {
            call_sequence_opt = self
                .call_sequences
                .get("default")
                .and_then(|opt| opt.as_ref());
        }

        if let Some(call_sequence) = call_sequence_opt {
            return self
                .fanout_with_call_sequence(
                    req_id,
                    start_at,
                    parent_chain,
                    graph_name,
                    call_sequence,
                )
                .await;
        }

        CALL_SEQUENCE_MISSING_WARN_ONCE.call_once(|| {
            warn!(
                "Call sequence missing for service {} and graph {}; falling back to default fanout logic",
                self.self_svc_name.as_str(),
                graph_name
            );
        });

        self.fanout_default(req_id, start_at, parent_chain, graph_name)
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
                    method_name: entry.method_name.to_string(),
                    graph_name: graph_name.to_string(),
                });

                // Set method name override for latency tracking
                request
                    .set_method_name_override(&entry.method_name)
                    .map_err(|e| {
                        Status::internal(format!("Failed to set method name override: {:?}", e))
                    })?;

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
            for (_child_svc, handle) in tasks {
                let rpc_result = handle
                    .await
                    .map_err(|e| Status::internal(format!("Task join error: {:?}", e)))?;
                rpc_result.map_err(|err| {
                    // error!(
                    //     "RPC to child service {} failed: {:?}",
                    //     child_svc.as_str(),
                    //     err
                    // );
                    err
                })?;
            }
        }

        Ok(())
    }

    async fn fanout_default(
        &self,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: &str,
    ) -> Result<(), Status> {
        let mut tasks = Vec::new();
        let mut parent_chain_for_children = parent_chain.clone();
        parent_chain_for_children.push(self.self_svc_name.clone());

        let parent_chain_metadata = encode_parent_chain(&parent_chain_for_children)?;

        let clients_guard = self.clients.read().await;
        for (child_svc_name, client) in clients_guard.iter() {
            if child_svc_name == &self.self_svc_name {
                continue;
            }

            if parent_chain.iter().any(|svc| svc == child_svc_name) {
                continue;
            }

            let probability = self
                .child_call_probabilities
                .get(child_svc_name)
                .copied()
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);

            if probability <= 0.0 {
                continue;
            }

            if probability < 1.0 && rand::random::<f64>() >= probability {
                continue;
            }

            let (method_to_call, _method_graph) = self
                .sample_method_for_child(child_svc_name, graph_name)
                .ok_or_else(|| {
                    Status::not_found(format!(
                        "Configuration error: Service {} has no method to call",
                        child_svc_name
                    ))
                })?;

            let mut client = client.clone();
            let mut request = Request::new(InvokeRequest {
                req_id,
                start_at,
                method_name: method_to_call.to_string(),
                graph_name: graph_name.to_string(),
            });

            // Set method name override for latency tracking
            request
                .set_method_name_override(&method_to_call)
                .map_err(|e| {
                    Status::internal(format!("Failed to set method name override: {:?}", e))
                })?;

            if let Some(ref metadata_value) = parent_chain_metadata {
                request
                    .metadata_mut()
                    .insert(PARENT_CHAIN_METADATA_KEY, metadata_value.clone());
            }

            let child = child_svc_name.clone();
            let handle = tokio::spawn(async move {
                client
                    .invoke(request)
                    .await
                    .map_err(|e| Status::internal(format!("RPC to child service failed: {:?}", e)))
            });
            tasks.push((child, handle));
        }
        drop(clients_guard);

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
        Ok(())
    }

    fn sample_method_for_child(
        &self,
        child_svc_name: &ServiceName,
        graph_name: &str,
    ) -> Option<(MethodId, Option<String>)> {
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
    /// The call sequences are loaded upfront for all graphs at startup.
    pub(crate) async fn fanout_with_user_call_sequence(
        &self,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: &str,
    ) -> Result<(), Status> {
        // Look up USER call sequence for this graph
        // graph_name might be in format "s-14677443" but graph_id in file is "S_14677443"
        // Try both formats
        let graph_id_variants = vec![
            graph_name.to_string(),
            graph_name.replace("s-", "S_"),
            graph_name.replace("-", "_"),
        ];

        let mut user_call_sequence_opt = None;
        for variant in &graph_id_variants {
            if let Some(seq) = self.user_call_sequences.get(variant) {
                user_call_sequence_opt = Some(seq);
                break;
            }
        }

        // If not found, try "default" as fallback
        if user_call_sequence_opt.is_none() {
            user_call_sequence_opt = self.user_call_sequences.get("default");
        }

        let user_call_sequence = user_call_sequence_opt.ok_or_else(|| {
            Status::not_found(format!(
                "USER call sequence not found for graph: {}",
                graph_name
            ))
        })?;

        // Use the pre-loaded USER call sequence for this graph
        self.fanout_with_call_sequence(
            req_id,
            start_at,
            parent_chain,
            graph_name,
            user_call_sequence,
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
