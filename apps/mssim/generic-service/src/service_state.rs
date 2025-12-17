use crate::bootstrap::ConnectionBootstrap;
use crate::busy_spin;
use crate::parent_chain::{encode_parent_chain, PARENT_CHAIN_METADATA_KEY};
use crate::service_replay::ReplaySpanExecutor;
use crate::service_stubs::{ReplayRequest, ServiceRequest};
use crate::RpcClient;
use anyhow::Result;
use sim_config::deployment::Deployment;
use sim_config::svc::{ServiceName, ServiceTraceConfig};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard};
use tonic::{Request, Status};
use tracing::{error, warn};

pub(crate) struct ServiceState {
    config: ServiceTraceConfig,
    clients: Arc<RwLock<HashMap<ServiceName, RpcClient>>>,
    child_call_probabilities: HashMap<ServiceName, f64>,
    self_svc_name: ServiceName,
    overshot_counter: AtomicUsize,
}

impl ServiceState {
    pub(crate) fn initialize(
        self_svc_name: ServiceName,
        config: ServiceTraceConfig,
        deployment: Deployment,
    ) -> Result<(Arc<Self>, Option<ConnectionBootstrap>)> {
        let child_weights = config.call_graph.callees_of(&self_svc_name);
        let child_call_probabilities = compute_child_probabilities(&child_weights);
        let clients = Arc::new(RwLock::new(HashMap::new()));
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
            child_call_probabilities,
            self_svc_name,
            overshot_counter: AtomicUsize::new(0),
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
    ) -> Result<(), Status> {
        let method_name = method_name.into();
        let latency_dist = self
            .config
            .method_latency
            .as_ref()
            .ok_or_else(|| Status::internal("Configuration error: method latency not configured"))?
            .get_method_dist(&method_name)
            .ok_or_else(|| Status::not_found("Method not found"))?;

        let total_latency_ms = latency_dist.sample(&mut rand::rng());

        let start_time = std::time::Instant::now();
        self.fanout(req_id, start_at, parent_chain).await?;
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

            let method_to_call = self
                .config
                .method_freq_map
                .as_ref()
                .and_then(|map| map.get_service(child_svc_name))
                .and_then(|sampler| Some(sampler.sample(&mut rand::rng()).to_string()))
                .ok_or_else(|| {
                    Status::not_found(format!(
                        "Configuration error: Service {} has no method to call",
                        child_svc_name
                    ))
                })?;

            let mut client = client.clone();
            let mut request = Request::new(ServiceRequest {
                req_id,
                start_at,
                method_name: method_to_call,
            });

            if let Some(ref metadata_value) = parent_chain_metadata {
                request
                    .metadata_mut()
                    .insert(PARENT_CHAIN_METADATA_KEY, metadata_value.clone());
            }

            let child = child_svc_name.clone();
            let handle = tokio::spawn(async move {
                client
                    .get_data(request)
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
                error!("RPC to child service {} failed", child_svc.as_str());
                err
            })?;
        }
        Ok(())
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
