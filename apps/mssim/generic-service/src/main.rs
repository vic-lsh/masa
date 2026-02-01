use anyhow::Result;
use masa::MethodId;
use service_stubs::service_client::ServiceClient;
use sim_config::deployment::Deployment;
use sim_config::svc::{CallGraphConfig, GraphId, ServiceName};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::current_thread_queue_len;
use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::{transport::Server, Request, Response, Status};
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use structopt::StructOpt;
use app_utils::{config::PolicyArgs, launch_masa_server};
use std::net::SocketAddr;

mod bootstrap;
mod core;
mod parent_chain;
mod service_replay;

use core::ServiceCore;

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
    ) -> Result<Self> {
        let (state, bootstrap) = ServiceCore::initialize(self_svc_name, config, deployment)?;
        if let Some(connection_task) = bootstrap {
            connection_task.spawn();
        }

        Ok(Self { state })
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

        self.state()
            .handle_method(
                method_name.clone(),
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
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing::level_filters::LevelFilter::INFO)
        .try_init();
}

#[derive(StructOpt, Debug, Clone)]
pub struct Args {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(long, env = "DEPLOYMENT_CONFIG_PATH", default_value = "config/deployment.json")]
    pub deployment_config: PathBuf,

    #[structopt(long, env = "SERVICE_NAME")]
    pub service_name: String,

    #[structopt(long, env = "SERVICE_PORT", default_value = "50051")]
    pub port: u16,

    #[structopt(long, env = "CALLGRAPHS_BASE_DIR", default_value = "/app/callgraphs")]
    pub callgraphs_base: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();
    launch_masa_server!(ServiceServer, args.policy, build_service, args)
}

async fn build_service(args: Args) -> Result<(AlibabaService, SocketAddr), Box<dyn std::error::Error>> {
    init_tracing();

    let svc_name = ServiceName::from_string(args.service_name);
    let config = CallGraphConfig::from_multi_callgraph_dir(&args.callgraphs_base, &svc_name)
        .expect("Failed to load call graph config");

    info!("Config parsed");

    let deployment = Deployment::read_from_file(&args.deployment_config).expect("Failed to parse deployment");

    let svc = AlibabaService::new(svc_name.clone(), config, deployment).await?;

    // Spawn a task that prints the queue length every second
    tokio::spawn(async {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        let start_time = Instant::now();
        loop {
            interval.tick().await;
            let queue_len = current_thread_queue_len();
            let elapsed = start_time.elapsed();
            info!(
                "current_thread_queue_len: {} (elapsed: {:?})",
                queue_len, elapsed
            );
        }
    });

    let addr = format!("0.0.0.0:{}", args.port).parse()?;
    info!("🚀 Generic Service {:?} listening on {}", svc_name, addr);
    
    Ok((svc, addr))
}

pub(crate) fn busy_spin(duration: std::time::Duration) {
    let start = std::time::Instant::now();
    while std::time::Instant::now() - start < duration {
        std::hint::spin_loop();
    }
}