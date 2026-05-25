use crate::bootstrap::ConnectionBootstrap;
use crate::busy_spin;
use crate::client_registry::{ClientLookup, ClientRegistry};
use crate::parent_chain::{encode_parent_chain, PARENT_CHAIN_METADATA_KEY};
use crate::service_replay::ReplaySpanExecutor;
use crate::service_stubs::{InvokeRequest, ReplayRequest};
use crate::RpcClient;
use anyhow::Result;
use masa::MasaRequestExt;
use masa::MethodId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use tokio::sync::RwLockReadGuard;
use tokio::task::JoinSet;
use tonic::{Request, Status};
use trace_config::deployment::Deployment;
use trace_config::svc::call_sequence::{
    CallSequence, CallSequenceVariant, MethodCallSequence, VariantChoice,
};
use trace_config::svc::{CallGraphConfig, GraphId, ServiceName};
use tracing::{info, warn};

pub(crate) struct ServiceCore {
    config: CallGraphConfig,
    clients: Arc<ClientRegistry>,
    self_svc_name: ServiceName,
    is_root_service: bool,
    overshot_counter: AtomicUsize,
    child_call_probabilities: HashMap<ServiceName, f64>,
}

impl ServiceCore {
    pub(crate) fn initialize(
        self_svc_name: ServiceName,
        config: CallGraphConfig,
        deployment: Deployment,
    ) -> Result<(Arc<Self>, Option<ConnectionBootstrap>)> {
        info!("Initializing service state for {}", self_svc_name.as_str());

        let is_root_service = self_svc_name.is_root_service();

        let child_weights = config.call_graph.callees_of(&self_svc_name);
        let child_call_probabilities = compute_child_probabilities(&child_weights);
        let clients = Arc::new(ClientRegistry::new());

        info!("Child services:");
        for child in child_weights.keys() {
            info!("{}", child.as_str());
        }

        info!(
            "Loaded call sequences for {} graphs",
            config.call_sequences.len()
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

        let state = Arc::new(ServiceCore {
            config,
            clients,
            self_svc_name,
            is_root_service,
            overshot_counter: AtomicUsize::new(0),
            child_call_probabilities,
        });

        Ok((state, bootstrap))
    }

    pub(crate) fn self_service_name(&self) -> &ServiceName {
        &self.self_svc_name
    }

    pub(crate) fn clients(&self) -> &Arc<ClientRegistry> {
        &self.clients
    }

    pub(crate) fn is_root_service(&self) -> bool {
        self.is_root_service
    }

    pub(crate) async fn handle_method(
        &self,
        method_id: MethodId,
        variant_id: Option<String>,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: &GraphId,
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
            self.fanout(
                req_id,
                start_at,
                parent_chain,
                graph_name,
                &method_id,
                variant_id,
            )
            .await?;
            let elapsed = start_time.elapsed();

            let remaining = total_latency_ms - (elapsed.as_millis() as f64);
            busy_spin(std::time::Duration::from_millis(remaining.max(1.0) as u64));
        }

        Ok(())
    }

    async fn handle_leaf_service(&self, total_latency_ms: f64) {
        const SPIN_FRACTION: f64 = 0.2;
        // Cap spin time so a few extreme-tail leaf samples (p99.9 reaching seconds
        // in some graphs) don't make the trace unreplayable above very low RPS.
        // Excess time is shifted into the sleep portion, preserving total service time.
        const MAX_SPIN_MS: f64 = 50.0;

        let spin_duration = (total_latency_ms * SPIN_FRACTION).min(MAX_SPIN_MS);
        let block_duration = total_latency_ms - spin_duration;

        busy_spin(std::time::Duration::from_millis(spin_duration as u64));
        tokio::time::sleep(std::time::Duration::from_millis(block_duration as u64)).await;
    }

    pub(crate) async fn fanout(
        &self,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: &GraphId,
        method_id: &MethodId,
        variant_id: Option<String>,
    ) -> Result<(), Status> {
        static CALL_SEQUENCE_MISSING_WARN_ONCE: Once = Once::new();

        let call_sequence = self
            .config
            .call_sequences
            .get(graph_name)
            .and_then(|opt| opt.as_ref())
            .ok_or_else(|| {
                CALL_SEQUENCE_MISSING_WARN_ONCE.call_once(|| {
                    warn!(
                        "Call sequence missing for service {} and graph {}",
                        self.self_svc_name.as_str(),
                        graph_name
                    );
                });
                Status::not_found(format!(
                    "Call sequence missing for service {} and graph {}",
                    self.self_svc_name.as_str(),
                    graph_name.as_str()
                ))
            })?;

        self.fanout_with_call_sequence(
            req_id,
            start_at,
            parent_chain,
            graph_name,
            method_id,
            variant_id.as_deref(),
            call_sequence,
        )
        .await
    }

    async fn fanout_with_call_sequence(
        &self,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: &GraphId,
        method_id: &MethodId,
        variant_id: Option<&str>,
        call_sequence: &CallSequence,
    ) -> Result<(), Status> {
        let lookup_method_id = self.call_sequence_method_id(method_id);
        let method_sequence = call_sequence.get_method(&lookup_method_id).ok_or_else(|| {
            Status::not_found(format!(
                "Call sequence method '{}' not found for graph '{}'",
                lookup_method_id,
                graph_name.as_str()
            ))
        })?;
        let selected_variant = select_call_sequence_variant(method_sequence, variant_id)?;
        let mut parent_chain_for_children = parent_chain.clone();
        parent_chain_for_children.push(self.self_svc_name.clone());
        let parent_chain_metadata = encode_parent_chain(&parent_chain_for_children)?;

        // Execute each step sequentially
        for step in &selected_variant.sequence {
            let mut tasks = JoinSet::new();

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

                let client = match self.clients.get_expect_ready(child_svc_name).await {
                    ClientLookup::Found(c) => c,
                    ClientLookup::StartupRace => {
                        warn!(
                            "Child service {} not found in clients map (bootstrap still in progress)",
                            child_svc_name.as_str()
                        );
                        continue;
                    }
                };

                let child_variant_id = sample_variant_choice(&entry.callee_variants);

                // Create request
                let mut client_clone = client.clone();
                let mut request = Request::new(InvokeRequest {
                    req_id,
                    start_at,
                    method_name: entry.method_name.to_string(),
                    graph_name: graph_name.as_str().to_string(),
                    variant_id: child_variant_id.unwrap_or_default(),
                });

                // Set method name override for latency tracking
                request
                    .set_method_name_override(&entry.method_name)
                    .map_err(|e| {
                        Status::internal(format!("Failed to set method name override: {:?}", e))
                    })?;
                request
                    .set_service_name_override(child_svc_name.as_str())
                    .map_err(|e| {
                        Status::internal(format!("Failed to set service name override: {:?}", e))
                    })?;

                if let Some(ref metadata_value) = parent_chain_metadata {
                    request
                        .metadata_mut()
                        .insert(PARENT_CHAIN_METADATA_KEY, metadata_value.clone());
                }

                // Spawn task for this child
                tasks.spawn(async move { client_clone.invoke(request).await });
            }

            // Wait for all tasks in this step to complete before proceeding to next step
            while let Some(task_result) = tasks.join_next().await {
                let rpc_result = task_result
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

    #[cfg(test)]
    pub(crate) async fn inject_client_for_test(&self, svc: ServiceName, client: RpcClient) {
        self.clients.insert_for_test(svc, client).await;
    }

    /// Use pre-loaded USER call sequence for root API.
    /// The call sequences are loaded upfront for all graphs at startup.
    pub(crate) async fn fanout_with_user_call_sequence(
        &self,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
        graph_name: &GraphId,
    ) -> Result<(), Status> {
        let user_call_sequence = self
            .config
            .call_sequences
            .get(graph_name)
            .and_then(|opt| opt.as_ref())
            .ok_or_else(|| {
                Status::not_found(format!(
                    "USER call sequence not found for graph: {}",
                    graph_name.as_str()
                ))
            })?;

        // Use the pre-loaded USER call sequence for this graph
        self.fanout_with_call_sequence(
            req_id,
            start_at,
            parent_chain,
            graph_name,
            &MethodId::from("USER"),
            None,
            user_call_sequence,
        )
        .await
    }

    fn call_sequence_method_id(&self, method_id: &MethodId) -> MethodId {
        if method_id == &MethodId::from("USER") {
            return MethodId::from("USER");
        }

        format!("{}::{}", self.self_svc_name.as_str(), method_id).into()
    }
}

fn select_call_sequence_variant<'a>(
    method_sequence: &'a MethodCallSequence,
    requested_variant: Option<&str>,
) -> Result<&'a CallSequenceVariant, Status> {
    if let Some(variant_id) = requested_variant.filter(|id| !id.is_empty()) {
        return method_sequence.get_variant(variant_id).ok_or_else(|| {
            Status::not_found(format!("Call sequence variant '{}' not found", variant_id))
        });
    }

    sample_weighted_variant(method_sequence.variants.values()).ok_or_else(|| {
        Status::not_found("Call sequence method has no variant with positive probability")
    })
}

fn sample_weighted_variant<'a, I>(variants: I) -> Option<&'a CallSequenceVariant>
where
    I: IntoIterator<Item = &'a CallSequenceVariant>,
{
    let variants: Vec<_> = variants.into_iter().collect();
    let total: f64 = variants
        .iter()
        .map(|variant| variant.probability.max(0.0))
        .sum();
    if total <= 0.0 {
        return None;
    }

    let mut threshold = rand::random::<f64>() * total;
    for variant in variants {
        threshold -= variant.probability.max(0.0);
        if threshold <= 0.0 {
            return Some(variant);
        }
    }
    None
}

fn sample_variant_choice(choices: &[VariantChoice]) -> Option<String> {
    let total: f64 = choices
        .iter()
        .map(|choice| choice.probability.max(0.0))
        .sum();
    if total <= 0.0 {
        return None;
    }

    let mut threshold = rand::random::<f64>() * total;
    for choice in choices {
        threshold -= choice.probability.max(0.0);
        if threshold <= 0.0 {
            return Some(choice.variant_id.clone());
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_stubs::{self, InvokeRequest, InvokeResponse};
    use masa::transport::LoadBalancedChannel;
    use masa::{MasaRequestExt, MethodId};
    use std::collections::HashMap;
    use tonic::async_trait;
    use tonic::transport::Server;
    use tonic::Request;
    use trace_config::svc::{
        call_sequence::{
            CallSequence, CallSequenceEntry, CallSequenceVariant, MethodCallSequence, VariantChoice,
        },
        CallGraphConfig, GraphId, ServiceName,
    };

    #[test]
    fn test_masa_overrides_available() {
        // This test confirms that MasaRequestExt is correctly imported and usable
        // within the generic-service crate, and that the header setting logic works.
        let mut req = Request::new(());

        req.set_service_name_override("test-service")
            .expect("failed to set service override");
        req.set_method_name_override("test-method")
            .expect("failed to set method override");

        assert_eq!(req.get_service_name_override().unwrap(), "test-service");
        assert_eq!(req.get_method_name_override().unwrap(), "test-method");
    }

    // --- Helpers ---

    struct MockSvc {
        tx: tokio::sync::mpsc::Sender<(String, String, String)>,
    }

    #[async_trait]
    impl service_stubs::service_server::Service for MockSvc {
        async fn invoke(
            &self,
            request: tonic::Request<InvokeRequest>,
        ) -> Result<tonic::Response<InvokeResponse>, tonic::Status> {
            let svc_override = request
                .get_service_name_override()
                .unwrap_or_default()
                .to_string();
            let method_override = request
                .get_method_name_override()
                .unwrap_or_default()
                .to_string();
            let variant_id = request.into_inner().variant_id;

            let _ = self
                .tx
                .send((svc_override, method_override, variant_id))
                .await;
            Ok(tonic::Response::new(InvokeResponse::default()))
        }
        async fn ping(
            &self,
            _: tonic::Request<service_stubs::PingRequest>,
        ) -> Result<tonic::Response<service_stubs::PingResponse>, tonic::Status> {
            Ok(tonic::Response::new(service_stubs::PingResponse::default()))
        }
        async fn replay(
            &self,
            _: tonic::Request<service_stubs::ReplayRequest>,
        ) -> Result<tonic::Response<service_stubs::ReplayResponse>, tonic::Status> {
            Ok(tonic::Response::new(
                service_stubs::ReplayResponse::default(),
            ))
        }
        async fn root(
            &self,
            _: tonic::Request<service_stubs::RootRequest>,
        ) -> Result<tonic::Response<service_stubs::RootResponse>, tonic::Status> {
            Ok(tonic::Response::new(service_stubs::RootResponse::default()))
        }
    }

    async fn spawn_mock_server() -> (u16, tokio::sync::mpsc::Receiver<(String, String, String)>) {
        let (tx, rx) = tokio::sync::mpsc::channel(1);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let incoming =
            tonic::transport::server::TcpIncoming::from_listener(listener, true, None).unwrap();

        tokio::spawn(async move {
            Server::builder()
                .add_service(service_stubs::service_server::ServiceServer::new(MockSvc {
                    tx,
                }))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        (port, rx)
    }

    async fn setup_core_with_config(
        config: CallGraphConfig,
        mock_port: u16,
        self_name: &str,
        target_name: &str,
    ) -> Arc<ServiceCore> {
        let self_svc = ServiceName::from_string(self_name.to_string());
        let child_svc = ServiceName::from_string(target_name.to_string());

        let client_channel =
            LoadBalancedChannel::new_from_service_name("127.0.0.1".to_string(), mock_port, 1).await;
        let client = crate::RpcClient::new(client_channel);

        let deployment = trace_config::deployment::Deployment::new();
        let (core, _) = ServiceCore::initialize(self_svc.clone(), config, deployment).unwrap();

        // Inject client
        core.inject_client_for_test(child_svc.clone(), client).await;
        core
    }

    // --- Tests ---

    fn single_variant_sequence(child_svc: &ServiceName, method: &str) -> CallSequence {
        let mut variants = HashMap::new();
        variants.insert(
            "v001".to_string(),
            CallSequenceVariant {
                variant_id: "v001".to_string(),
                probability: 1.0,
                sequence: vec![vec![CallSequenceEntry {
                    service_name: child_svc.clone(),
                    method_name: method.to_string().into(),
                    callee_variants: Vec::new(),
                }]],
            },
        );

        let mut methods = HashMap::new();
        methods.insert(
            MethodId::from("sender::parent-method"),
            MethodCallSequence { variants },
        );
        CallSequence { methods }
    }

    #[tokio::test]
    async fn test_overrides_integration() {
        // 1. Setup Mock Server
        let (port, mut rx) = spawn_mock_server().await;

        // 2. Setup Config & Core
        let graph_id = GraphId::from_string("test-graph".to_string());
        let child_svc = ServiceName::from_string("receiver".to_string());

        let call_seq = single_variant_sequence(&child_svc, "test-method");

        let mut call_sequences = HashMap::new();
        call_sequences.insert(graph_id.clone(), Some(call_seq));

        let config = CallGraphConfig {
            call_sequences,
            method_latency: None,
            method_freq_map: None,
            call_graph: trace_config::svc::call_graph::CallGraph::default(),
        };

        let core = setup_core_with_config(config, port, "sender", "receiver").await;

        // 3. Execute
        core.fanout(
            1,
            0,
            vec![],
            &graph_id,
            &MethodId::from("parent-method"),
            Some("v001".to_string()),
        )
        .await
        .unwrap();

        // 4. Assert
        let (svc_over, meth_over, variant_id) = rx.recv().await.unwrap();
        assert_eq!(svc_over, "receiver");
        assert_eq!(meth_over, "test-method");
        assert_eq!(variant_id, "");
    }

    #[tokio::test]
    async fn test_fanout_propagates_child_variant() {
        // 1. Setup Mock Server
        let (port, mut rx) = spawn_mock_server().await;

        // 2. Setup Config with a deterministic child variant mapping.
        let graph_id = GraphId::from_string("variant-graph".to_string());
        let child_svc = ServiceName::from_string("receiver".to_string());

        let mut variants = HashMap::new();
        variants.insert(
            "v001".to_string(),
            CallSequenceVariant {
                variant_id: "v001".to_string(),
                probability: 1.0,
                sequence: vec![vec![CallSequenceEntry {
                    service_name: child_svc.clone(),
                    method_name: MethodId::from("variant-method"),
                    callee_variants: vec![VariantChoice {
                        variant_id: "v007".to_string(),
                        probability: 1.0,
                    }],
                }]],
            },
        );
        let mut methods = HashMap::new();
        methods.insert(
            MethodId::from("sender::parent-method"),
            MethodCallSequence { variants },
        );
        let call_seq = CallSequence { methods };

        let mut call_sequences = HashMap::new();
        call_sequences.insert(graph_id.clone(), Some(call_seq));

        let config = CallGraphConfig {
            call_sequences,
            method_latency: None,
            method_freq_map: None,
            call_graph: trace_config::svc::call_graph::CallGraph::default(),
        };

        let core = setup_core_with_config(config, port, "sender", "receiver").await;

        // 3. Execute
        core.fanout(
            1,
            0,
            vec![],
            &graph_id,
            &MethodId::from("parent-method"),
            Some("v001".to_string()),
        )
        .await
        .unwrap();

        // 4. Assert
        let (svc_over, meth_over, variant_id) = rx.recv().await.unwrap();
        assert_eq!(svc_over, "receiver");
        assert_eq!(meth_over, "variant-method");
        assert_eq!(variant_id, "v007");
    }

    #[tokio::test]
    async fn test_fanout_uses_requested_parent_variant() {
        let (port, mut rx) = spawn_mock_server().await;
        let graph_id = GraphId::from_string("requested-variant-graph".to_string());
        let child_svc = ServiceName::from_string("receiver".to_string());

        let mut variants = HashMap::new();
        variants.insert(
            "v001".to_string(),
            CallSequenceVariant {
                variant_id: "v001".to_string(),
                probability: 0.99,
                sequence: vec![vec![CallSequenceEntry {
                    service_name: child_svc.clone(),
                    method_name: MethodId::from("wrong-method"),
                    callee_variants: Vec::new(),
                }]],
            },
        );
        variants.insert(
            "v002".to_string(),
            CallSequenceVariant {
                variant_id: "v002".to_string(),
                probability: 0.01,
                sequence: vec![vec![CallSequenceEntry {
                    service_name: child_svc.clone(),
                    method_name: MethodId::from("requested-method"),
                    callee_variants: vec![VariantChoice {
                        variant_id: "child-v002".to_string(),
                        probability: 1.0,
                    }],
                }]],
            },
        );

        let mut methods = HashMap::new();
        methods.insert(
            MethodId::from("sender::parent-method"),
            MethodCallSequence { variants },
        );
        let mut call_sequences = HashMap::new();
        call_sequences.insert(graph_id.clone(), Some(CallSequence { methods }));

        let config = CallGraphConfig {
            call_sequences,
            method_latency: None,
            method_freq_map: None,
            call_graph: trace_config::svc::call_graph::CallGraph::default(),
        };

        let core = setup_core_with_config(config, port, "sender", "receiver").await;
        core.fanout(
            1,
            0,
            vec![],
            &graph_id,
            &MethodId::from("parent-method"),
            Some("v002".to_string()),
        )
        .await
        .unwrap();

        let (svc_over, meth_over, variant_id) = rx.recv().await.unwrap();
        assert_eq!(svc_over, "receiver");
        assert_eq!(meth_over, "requested-method");
        assert_eq!(variant_id, "child-v002");
    }
}
