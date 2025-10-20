use anyhow::{Context, Result};
use masa::Context as MasaContext;
use service_stubs::service_client::ServiceClient;
use sim_config::deployment::{Deployment, ServiceDiscoveryInfo};
use sim_config::svc::{ServiceName, ServiceTraceConfig};
use std::collections::HashMap;
use std::env;
use std::str::FromStr;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;
use tokio::time::{sleep, Duration};
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::{transport::Server, Request, Response, Status};
use tracing::level_filters::LevelFilter;
use tracing::{error, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub mod service_stubs {
    tonic::include_proto!("service");
}

use service_stubs::service_server::{Service, ServiceServer};
use service_stubs::{
    local_span::SpanType, span::Kind, PingRequest, PingResponse, ReplayRequest, ReplayResponse,
    ResponseStatus, RootRequest, RootResponse, ServiceRequest, ServiceResponse,
};

type RpcClient = ServiceClient<LoadBalancedChannel>;

const PARENT_CHAIN_METADATA_KEY: &str = "parent-chain";
const PARENT_CHAIN_DELIMITER: char = '>';

#[allow(dead_code)]
struct AlibabaService {
    config: ServiceTraceConfig,
    clients: HashMap<ServiceName, RpcClient>,
    deployment: Deployment,
    self_svc_name: ServiceName,
    overshot_counter: AtomicUsize,
}

impl AlibabaService {
    pub async fn new(
        self_svc_name: ServiceName,
        config: ServiceTraceConfig,
        deployment: Deployment,
    ) -> Result<Self> {
        let children = config.call_graph.callees_of(&self_svc_name);

        println!("Connecting to children: {:?}", children);
        let clients = Self::connect_to_children(children, &deployment).await?;
        println!("Children connected");

        Ok(AlibabaService {
            config,
            clients,
            deployment,
            self_svc_name,
            overshot_counter: AtomicUsize::new(0),
        })
    }

    async fn connect_to_children(
        children: impl IntoIterator<Item = ServiceName>,
        deployment: &Deployment,
    ) -> Result<HashMap<ServiceName, RpcClient>> {
        let mut clients = HashMap::new();
        let children_it = children.into_iter();
        for child_svc_name in children_it {
            let svc_info = deployment.services.get(&child_svc_name).with_context(|| {
                format!("Child service {} not found in deployment", child_svc_name)
            })?;
            let client = Self::connect_to_child_retried(svc_info).await?;
            println!("Connected to child service {}", child_svc_name);
            clients.insert(child_svc_name.clone(), client);
        }
        Ok(clients)
    }

    async fn connect_to_child_retried(svc_info: &ServiceDiscoveryInfo) -> Result<RpcClient> {
        let ip = svc_info.ip.clone();
        let channel = LoadBalancedChannel::new(
            ip,
            svc_info.port,
            svc_info
                .replicas
                .try_into()
                .expect("Replica count too high"),
        )
        .await;

        Ok(ServiceClient::new(channel))
    }

    fn encode_parent_chain(
        chain: &[ServiceName],
    ) -> Result<Option<MetadataValue<tonic::metadata::Ascii>>, Status> {
        if chain.is_empty() {
            return Ok(None);
        }

        let delimiter = PARENT_CHAIN_DELIMITER.to_string();
        let encoded = chain
            .iter()
            .map(|svc| svc.as_str())
            .collect::<Vec<_>>()
            .join(&delimiter);

        MetadataValue::from_str(encoded.as_str())
            .map(Some)
            .map_err(|_| Status::internal("Failed to encode parent chain metadata"))
    }

    fn decode_parent_chain(metadata: &MetadataMap) -> Result<Vec<ServiceName>, Status> {
        let Some(value) = metadata.get(PARENT_CHAIN_METADATA_KEY) else {
            return Ok(Vec::new());
        };

        let parents_str = value
            .to_str()
            .map_err(|_| Status::invalid_argument("Parent chain metadata is not valid ASCII"))?;

        if parents_str.is_empty() {
            return Ok(Vec::new());
        }

        let parents = parents_str
            .split(PARENT_CHAIN_DELIMITER)
            .filter(|name| !name.is_empty())
            .map(|name| ServiceName::from_string(name.to_string()))
            .collect();

        Ok(parents)
    }
}

#[tonic::async_trait]
impl Service for AlibabaService {
    async fn get_data(
        &self,
        request: Request<ServiceRequest>,
    ) -> Result<Response<ServiceResponse>, Status> {
        let parent_chain = Self::decode_parent_chain(request.metadata())?;
        let _req = request.into_inner();
        let method_name = _req.method_name;

        self.handle_method(
            method_name.clone(),
            _req.req_id,
            _req.start_at,
            parent_chain,
        )
        .await?;

        Ok(Response::new(ServiceResponse {
            calls: vec![],
            method_name: method_name,
        }))
    }

    async fn root(&self, _request: Request<RootRequest>) -> Result<Response<RootResponse>, Status> {
        // TODO: remove this coupling with alibaba's data
        const ROOT_SVC_NAME: &'static str = "user";

        if self.self_svc_name.as_str() != ROOT_SVC_NAME {
            return Err(Status::permission_denied(format!(
                "Root endpoint can only be called on service {}, not {}",
                ROOT_SVC_NAME,
                self.self_svc_name.as_str()
            )));
        }
        let _req = _request.into_inner();
        // All the root service does is calling into internal services
        self.fanout(_req.req_id, _req.start_at, Vec::new()).await?;

        Ok(Response::new(RootResponse {}))
    }

    async fn ping(&self, _request: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        Ok(Response::new(PingResponse {}))
    }

    async fn replay(
        &self,
        _request: tonic::Request<ReplayRequest>,
    ) -> Result<Response<ReplayResponse>, Status> {
        let req = _request.into_inner();

        // if req.exclude_queue_latency > req.slo {
        //     return Err(Status::cancelled("Request latency too high (> 50 ms)"));
        // }

        let start = Instant::now();

        let spans = req.spans;

        use std::convert::TryFrom;
        for span in spans {
            if let Some(kind) = span.kind {
                match kind {
                    Kind::LocalSpan(single_span) => match SpanType::try_from(single_span.r#type) {
                        Ok(SpanType::Compute) => {
                            busy_spin(Duration::from_micros(single_span.val));
                        }
                        Ok(SpanType::Block) => {
                            sleep(Duration::from_micros(single_span.val)).await;
                        }
                        Ok(SpanType::Unknown) => {
                            warn!("Unknown span type, skipping");
                        }
                        Err(_) => {
                            warn!("Invalid span type, skipping");
                        }
                    },
                    Kind::ChildSpans(span_vector) => {
                        let child_name = span_vector.name;
                        let child_channel = self
                            .clients
                            .get(&ServiceName::from_string(child_name.clone()))
                            .ok_or(Status::not_found(format!(
                                "Child service {} not found",
                                child_name
                            )))?;

                        let mut child_req = Request::new(ReplayRequest {
                            req_id: req.req_id,
                            exclude_queue_latency: req.exclude_queue_latency,
                            slo: req.slo,
                            start_at: req.start_at,
                            deadline: req.deadline,
                            spans: span_vector.spans.clone(),
                        });

                        let ctx = {
                            MasaContext::new(
                                "replay".to_string(),
                                0,
                                req.req_id,
                                req.slo,
                                0,
                                req.start_at,
                                req.deadline,
                            )
                        };

                        child_req.metadata_mut().insert_ctx("ctx", &ctx);

                        let mut child_channel = child_channel.clone();
                        match child_channel.replay(child_req).await {
                            Ok(_response) => {
                                // Child call succeeded
                            }
                            Err(e) => {
                                return Err(Status::internal(format!(
                                    "RPC to child service {} dropped or failed: {:?}",
                                    child_name, e
                                )));
                            }
                        }
                    }
                }
            }
        }

        let elapsed = start.elapsed();
        if elapsed.as_millis() > req.exclude_queue_latency as u128 {
            let old = self
                .overshot_counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if old % 10 == 0 {
                warn!(
                    "Warning {}: processing took longer ({:?}) than request latency ({} us)",
                    old, elapsed, req.exclude_queue_latency
                );
            }
        } else if elapsed.as_micros() > req.slo as u128 {
            return Err(Status::cancelled("Processing took more than SLO"));
        }

        Ok(Response::new(ReplayResponse {
            req_id: req.req_id,
            status: ResponseStatus::Ok as i32,
        }))
    }
}

fn busy_spin(duration: Duration) {
    let start = std::time::Instant::now();
    while std::time::Instant::now() - start < duration {
        // Busy spin
    }
}

impl AlibabaService {
    async fn handle_method(
        &self,
        method_name: String,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
    ) -> Result<(), Status> {
        let name = method_name.into();
        let latency_dist = self
            .config
            .method_latency
            .as_ref()
            .ok_or(Status::internal(
                "Configuration error: method latency not configured",
            ))?
            .get_method_dist(&name)
            .ok_or(Status::not_found("Method not found"))?;

        let total_latency_ms = latency_dist.sample(&mut rand::rng());

        let start = Instant::now();
        self.fanout(req_id, start_at, parent_chain).await?;
        let elapsed = start.elapsed();

        let remaining = total_latency_ms - (elapsed.as_millis() as f64);
        if remaining > 0.0 {
            busy_spin(Duration::from_millis(remaining as u64));
        } else {
            warn!(
                "Warning: fanout took longer ({:?}) than total latency ({:.2} ms)",
                elapsed, total_latency_ms
            );
        }

        Ok(())
    }

    async fn fanout(
        &self,
        req_id: u64,
        start_at: u64,
        parent_chain: Vec<ServiceName>,
    ) -> Result<(), Status> {
        let mut tasks = Vec::new();
        let mut parent_chain_for_children = parent_chain.clone();
        parent_chain_for_children.push(self.self_svc_name.clone());

        let parent_chain_metadata = Self::encode_parent_chain(&parent_chain_for_children)?;

        for (child_svc_name, client) in &self.clients {
            // Disallow self-edge
            if child_svc_name == &self.self_svc_name {
                continue;
            }

            // Disallow cycle
            if parent_chain.iter().any(|svc| svc == child_svc_name) {
                continue;
            }

            let method_to_call = self
                .config
                .method_freq_map
                .as_ref()
                .unwrap()
                .get_service(child_svc_name)
                .and_then(|sampler| Some(sampler.sample(&mut rand::rng()).to_string()))
                .ok_or(Status::not_found(format!(
                    "Configuration error: Service {child_svc_name} has no method to call"
                )))?;

            let mut client = client.clone();
            let mut request = tonic::Request::new(ServiceRequest {
                req_id: req_id.clone(),
                start_at: start_at.clone(),
                method_name: method_to_call,
            });

            let ctx = {
                let slo = 50_000;
                let deadline = start_at + slo;
                MasaContext::new("fanout".to_string(), 0, req_id, slo, 0, start_at, deadline)
            };

            request.metadata_mut().insert_ctx("ctx", &ctx);

            if let Some(ref metadata_value) = parent_chain_metadata {
                request
                    .metadata_mut()
                    .insert(PARENT_CHAIN_METADATA_KEY, metadata_value.clone());
            }

            let handle = tokio::spawn(async move {
                client
                    .get_data(request)
                    .await
                    .map_err(|e| Status::internal(format!("RPC to child service failed: {:?}", e)))
            });
            tasks.push((child_svc_name, handle));
        }
        for (child_svc, handle) in tasks {
            let rpc_result = handle
                .await
                .map_err(|e| Status::internal(format!("Task join error: {:?}", e)))?;
            rpc_result.map_err(|e| {
                error!("RPC to child service {} failed", child_svc);
                e
            })?;
        }
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // TODO: make log level configurable
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(LevelFilter::INFO)
        .init();

    let deployment_path_str =
        env::var("DEPLOYMENT_CONFIG_PATH").unwrap_or_else(|_| "config/deployment.json".to_string());
    let config_dir_str = env::var("CONFIG_PATH").unwrap_or_else(|_| "config/".to_string());
    let service_name = env::var("SERVICE_NAME").expect("Failed to get SERVICE_NAME");
    let port = env::var("SERVICE_PORT").unwrap_or_else(|_| "50051".to_string());

    let path = config_dir_str.into();

    let svc_name = ServiceName::from_string(service_name);

    // NOTE: HACK. Either make the root service name configurable, or
    // configure the config files such that the root service has the same schema.
    //
    // Right now, the root service is "user" in Alibaba traces.
    let config = {
        const ROOT_SVC_NAME: &'static str = "user";
        let root_svc_name = ServiceName::from_string(ROOT_SVC_NAME.to_string());
        let svc_name_for_config = if svc_name == root_svc_name {
            None
        } else {
            Some(svc_name.clone())
        };

        ServiceTraceConfig::from_config_dir(&path, svc_name_for_config)
            .expect("Loading config should succeed")
    };

    println!("Config parsed");

    let deployment_path = deployment_path_str.into();
    let deployment = Deployment::read_from_file(&deployment_path)?;

    let svc = AlibabaService::new(svc_name.clone(), config, deployment).await?;

    let addr = format!("0.0.0.0:{}", port).parse()?;
    println!("🚀 Generic Service {:?} listening on {}", svc_name, addr);

    Server::builder()
        .add_service(ServiceServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}
