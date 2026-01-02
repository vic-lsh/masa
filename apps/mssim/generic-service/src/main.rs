use anyhow::Result;
use rand::Rng;
use rand_distr::Exp;
use service_stubs::service_client::ServiceClient;
use sim_config::deployment::Deployment;
use sim_config::svc::MethodId;
use sim_config::svc::{ServiceName, ServiceTraceConfig};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::{transport::Server, Request, Response, Status};
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod bootstrap;
mod parent_chain;
mod service_replay;
mod service_state;

use service_state::ServiceState;

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
    state: Arc<ServiceState>,
}

impl AlibabaService {
    pub async fn new(
        self_svc_name: ServiceName,
        config: ServiceTraceConfig,
        deployment: Deployment,
        config_dir: std::path::PathBuf,
    ) -> Result<Self> {
        let (state, bootstrap) =
            ServiceState::initialize(self_svc_name, config, deployment, config_dir)?;
        if let Some(connection_task) = bootstrap {
            connection_task.spawn();
        }

        Ok(Self { state })
    }

    fn state(&self) -> &ServiceState {
        &self.state
    }
}

#[tonic::async_trait]
impl Service for AlibabaService {
    async fn invoke(
        &self,
        request: Request<InvokeRequest>,
    ) -> Result<Response<InvokeResponse>, Status> {
        let parent_chain = parent_chain::decode_parent_chain(request.metadata())?;
        let request = request.into_inner();
        let method_name: MethodId = request.method_name.into();
        let graph_name = request.graph_name.as_str();

        self.state()
            .handle_method(
                method_name.clone(),
                request.req_id,
                request.start_at,
                parent_chain,
                Some(graph_name),
            )
            .await?;

        Ok(Response::new(InvokeResponse {
            calls: vec![],
            method_name: method_name.into(),
        }))
    }

    async fn root(&self, request: Request<RootRequest>) -> Result<Response<RootResponse>, Status> {
        const ROOT_SVC_NAME: &str = "user";
        let root_check = self
            .state()
            .self_service_name()
            .as_str()
            .starts_with(ROOT_SVC_NAME);

        if !root_check {
            return Err(Status::permission_denied(format!(
                "Root endpoint can only be called on service start with {}, not {}",
                ROOT_SVC_NAME,
                self.state().self_service_name().as_str()
            )));
        }

        let request = request.into_inner();
        let graph_name = request.graph_name.as_str();

        // Root uses pre-loaded USER call sequence (loaded at startup)
        self.state()
            .fanout_with_user_call_sequence(
                request.req_id,
                request.start_at,
                Vec::new(),
                graph_name,
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

fn load_service_config(
    config_dir: std::path::PathBuf,
    svc_name: &ServiceName,
) -> ServiceTraceConfig {
    const ROOT_SVC_NAME: &str = "user";

    // check svc name start with ROOT_SVC_NAME
    let svc_name_for_config = if svc_name.as_str().starts_with(ROOT_SVC_NAME) {
        None
    } else {
        Some(svc_name.clone())
    };

    ServiceTraceConfig::from_config_dir(&config_dir, svc_name_for_config)
        .expect("Loading config should succeed")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let deployment_path =
        env::var("DEPLOYMENT_CONFIG_PATH").unwrap_or_else(|_| "config/deployment.json".to_string());
    let config_dir = env::var("CONFIG_PATH").unwrap_or_else(|_| "config/".to_string());
    let service_name = env::var("SERVICE_NAME").expect("Failed to get SERVICE_NAME");
    let port = env::var("SERVICE_PORT").unwrap_or_else(|_| "50051".to_string());

    let config_path: PathBuf = config_dir.into();
    let svc_name = ServiceName::from_string(service_name);
    let config = load_service_config(config_path.clone(), &svc_name);
    info!("Config parsed");

    let deployment_path = deployment_path.into();
    let deployment =
        Deployment::read_from_file(&deployment_path).expect("Failed to parse deployment");

    let svc = AlibabaService::new(svc_name.clone(), config, deployment, config_path).await?;

    let addr = format!("0.0.0.0:{}", port).parse()?;
    info!("🚀 Generic Service {:?} listening on {}", svc_name, addr);

    Server::builder()
        .add_service(ServiceServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}

pub(crate) fn busy_spin(duration: std::time::Duration) {
    let start = std::time::Instant::now();
    while std::time::Instant::now() - start < duration {
        std::hint::spin_loop();
    }
}

/// Samples from an exponential distribution with the given rate parameter (lambda).
///
/// # Arguments
/// * `rate` - The rate parameter (lambda) of the exponential distribution.
///            Must be positive. The mean of the distribution is 1/rate.
///
/// # Returns
/// A sample from the exponential distribution.
///
/// # Panics
/// Panics if rate is not positive or if the distribution cannot be created.
pub(crate) fn sample_exponential(rate: f64) -> f64 {
    let dist = Exp::new(rate).expect("Failed to create exponential distribution");
    let mut rng = rand::thread_rng();
    rng.sample(dist)
}
