use anyhow::Result;
use masa::transport::LoadBalancedChannel;
use masa::MethodId;
use service_stubs::service_client::ServiceClient;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(not(feature = "sched_mt"))]
use tokio::runtime::current_thread_queue_len;
use tonic::{transport::Server, Request, Response, Status};
use trace_config::deployment::Deployment;
use trace_config::svc::{CallGraphConfig, GraphId, ServiceName};
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod bootstrap;
mod client_registry;
mod core;
mod parent_chain;
mod service_replay;

use core::ServiceCore;

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(300);

pub mod service_stubs {
    tonic::include_proto!("service");
}

use service_stubs::service_server::{Service, ServiceServer};
use service_stubs::{
    InvokeRequest, InvokeResponse, PingRequest, PingResponse, ReplayRequest, ReplayResponse,
    ResponseStatus, RootRequest, RootResponse,
};

pub(crate) type RpcClient = ServiceClient<LoadBalancedChannel>;

struct AlibabaService {
    state: Arc<ServiceCore>,
}

impl AlibabaService {
    pub async fn new(
        self_svc_name: ServiceName,
        config: CallGraphConfig,
        deployment: Deployment,
    ) -> Result<(Self, Option<bootstrap::ConnectionBootstrapTask>)> {
        let (state, bootstrap) = ServiceCore::initialize(self_svc_name, config, deployment)?;
        // Spawn bootstrap concurrently (not inline) so pairs of services that
        // call each other don't deadlock waiting for each other's Channel::new.
        // The fanout path panics on missing clients only after bootstrap signals
        // done, so startup-race misses are warnings, post-bootstrap misses fail.
        let connection_task = bootstrap.map(|task| task.spawn());

        Ok((Self { state }, connection_task))
    }

    fn state(&self) -> &ServiceCore {
        &self.state
    }
}

#[tonic::async_trait]
impl Service for AlibabaService {
    async fn invoke(
        &self,
        request: Request<InvokeRequest>,
    ) -> Result<Response<InvokeResponse>, Status> {
        if self.state().is_root_service() {
            return Err(Status::permission_denied(
                "Root services cannot receive Invoke requests. Use the Root endpoint instead.",
            ));
        }

        let parent_chain = parent_chain::decode_parent_chain(request.metadata())?;
        let request = request.into_inner();
        let method_name: MethodId = request.method_name.into();
        let graph_name = GraphId::from_string(request.graph_name);
        let variant_id = (!request.variant_id.is_empty()).then_some(request.variant_id);

        self.state()
            .handle_method(
                method_name.clone(),
                variant_id,
                request.req_id,
                request.start_at,
                parent_chain,
                &graph_name,
            )
            .await?;

        Ok(Response::new(InvokeResponse {
            calls: vec![],
            method_name: method_name.into(),
        }))
    }

    async fn root(&self, request: Request<RootRequest>) -> Result<Response<RootResponse>, Status> {
        if !self.state().is_root_service() {
            return Err(Status::permission_denied(format!(
                "Root endpoint can only be called on service start with \"user\", not {}",
                self.state().self_service_name().as_str()
            )));
        }

        let request = request.into_inner();
        let graph_name = GraphId::from_string(request.graph_name);

        // Root uses pre-loaded USER call sequence (loaded at startup)
        self.state()
            .fanout_with_user_call_sequence(
                request.req_id,
                request.start_at,
                Vec::new(),
                &graph_name,
            )
            .await?;

        Ok(Response::new(RootResponse {
            req_id: request.req_id,
        }))
    }

    async fn ping(&self, _request: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        Ok(Response::new(PingResponse {}))
    }

    async fn replay(
        &self,
        request: Request<ReplayRequest>,
    ) -> Result<Response<ReplayResponse>, Status> {
        let req = request.into_inner();
        let start = Instant::now();

        self.state().execute_replay(&req).await?;

        let elapsed = start.elapsed();
        self.state().evaluate_replay_timing(elapsed, &req)?;

        Ok(Response::new(ReplayResponse {
            req_id: req.req_id,
            status: ResponseStatus::Ok as i32,
        }))
    }
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing::level_filters::LevelFilter::INFO)
        .init();
}

#[cfg_attr(feature = "sched_mt", tokio::main)]
#[cfg_attr(not(feature = "sched_mt"), tokio::main(flavor = "current_thread"))]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let deployment_path =
        env::var("DEPLOYMENT_CONFIG_PATH").unwrap_or_else(|_| "config/deployment.json".to_string());
    let service_name = env::var("SERVICE_NAME").expect("Failed to get SERVICE_NAME");
    let port = env::var("SERVICE_PORT").unwrap_or_else(|_| "50051".to_string());

    // Get callgraphs base directory from environment variable, default to /app/callgraphs
    let callgraphs_base = env::var("CALLGRAPHS_BASE_DIR")
        .map(|s| PathBuf::from(s))
        .unwrap_or_else(|_| PathBuf::from("/app/callgraphs"));

    let svc_name = ServiceName::from_string(service_name);
    let config = CallGraphConfig::from_multi_callgraph_dir(&callgraphs_base, &svc_name)
        .expect("Failed to load call graph config");

    info!("Config parsed");

    let deployment_path = deployment_path.into();
    let deployment =
        Deployment::read_from_file(&deployment_path).expect("Failed to parse deployment");

    let (svc, connection_task) = AlibabaService::new(svc_name.clone(), config, deployment).await?;

    // Wait for bootstrap to finish before serving so we don't absorb a flood
    // of "startup race" warnings. Use a timeout as a safety net: if bootstrap
    // is genuinely stuck (e.g. mutual-call deadlock), we force-flip the
    // ready flag and serve anyway — post-flag fanout misses will panic,
    // making the failure observable instead of silently producing bogus
    // goodput.
    if let Some(task) = connection_task {
        let registry = Arc::clone(svc.state().clients());
        match tokio::time::timeout(BOOTSTRAP_TIMEOUT, task.into_handle()).await {
            Ok(Ok(())) => info!("Bootstrap completed"),
            Ok(Err(join_err)) => panic!("Bootstrap task panicked: {join_err}"),
            Err(_elapsed) => {
                tracing::warn!(
                    "Bootstrap did not complete within {:?}; forcing ready \
                     so further fanout misses will panic instead of silently succeeding",
                    BOOTSTRAP_TIMEOUT
                );
                registry.force_ready();
            }
        }
    }

    // Spawn a task that prints the queue length every second
    let queue_monitor_task = tokio::spawn(async {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        let start_time = Instant::now();
        loop {
            interval.tick().await;
            #[cfg(not(feature = "sched_mt"))]
            {
                let queue_len = current_thread_queue_len();
                let elapsed = start_time.elapsed();
                info!(
                    "current_thread_queue_len: {} (elapsed: {:?})",
                    queue_len, elapsed
                );
            }
            #[cfg(feature = "sched_mt")]
            let _ = start_time.elapsed();
        }
    });

    let addr = format!("0.0.0.0:{}", port).parse()?;
    info!("🚀 Generic Service {:?} listening on {}", svc_name, addr);

    Server::builder()
        .add_service(ServiceServer::new(svc))
        .serve(addr)
        .await?;
    queue_monitor_task.abort();
    let _ = queue_monitor_task.await;

    Ok(())
}

pub(crate) fn busy_spin(duration: std::time::Duration) {
    let start = std::time::Instant::now();
    while std::time::Instant::now() - start < duration {
        std::hint::spin_loop();
    }
}
