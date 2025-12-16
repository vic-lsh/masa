use anyhow::Result;
use rand::Rng;
use rand_distr::Exp;
use service_stubs::service_client::ServiceClient;
use sim_config::deployment::Deployment;
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
    PingRequest, PingResponse, ReplayRequest, ReplayResponse, ResponseStatus, RootRequest,
    RootResponse, ServiceRequest, ServiceResponse,
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

impl AlibabaService {
    async fn handle_ms_666(&self, _request: ServiceRequest) {
        use rand::Rng;

        let exp_sample_ms = {
            let mut rng = rand::thread_rng();
            let exp_sample_us = (-70_000.0 * rng.gen::<f64>().ln()).max(0.0); // Exponential sample in microseconds, mean 70_000us
            let exp_sample_ms = (exp_sample_us / 1000.0) as u64;
            exp_sample_ms
        };

        let busy_spin_duration = exp_sample_ms / 5u64;
        let sleep_duration = exp_sample_ms - busy_spin_duration;


        busy_spin(std::time::Duration::from_millis(busy_spin_duration));
        tokio::time::sleep(std::time::Duration::from_millis(sleep_duration)).await;

        // tokio::time::sleep(std::time::Duration::from_millis(exp_sample_ms)).await;

        // busy_spin(std::time::Duration::from_millis(4));
        // tokio::time::sleep(std::time::Duration::from_millis(18)).await;
    }

    async fn handle_ms_56394(&self, _request: ServiceRequest) {
        busy_spin(std::time::Duration::from_millis(10));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tonic::async_trait]
impl Service for AlibabaService {
    async fn get_data(
        &self,
        request: Request<ServiceRequest>,
    ) -> Result<Response<ServiceResponse>, Status> {
        let metadata = request.metadata().clone();

        let parent_chain = parent_chain::decode_parent_chain(request.metadata())?;
        let request = request.into_inner();
        let method_name = request.method_name.clone();
        let graph_name = request.graph_name.as_str();

        match self.state().self_service_name().as_str() {
            "ms-666" => self.handle_ms_666(request).await,
            "ms-56394" => self.handle_ms_56394(request).await,
            _ => return Err(Status::invalid_argument("Invalid service name")),
        };

        // self.state()
        //     .handle_method(
        //         method_name.clone(),
        //         request.req_id,
        //         request.start_at,
        //         parent_chain,
        //         Some(graph_name),
        //     )
        //     .await?;

        Ok(Response::new(ServiceResponse {
            calls: vec![],
            method_name,
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
        
        if graph_name == "s-14677443" {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;

            let svc_name = ServiceName::from_string("ms-56394".to_string());
            let mut client = self.state.clients.read().await.get(&svc_name).unwrap().clone();
            let req = Request::new(ServiceRequest {
                req_id: request.req_id,
                start_at: request.start_at,
                method_name: "get_data".to_string(),
                graph_name: graph_name.to_string(),
            });
            client.get_data(req).await?;

            let svc_name = ServiceName::from_string("ms-666".to_string());
            let mut client = self.state.clients.read().await.get(&svc_name).unwrap().clone();
            let req = Request::new(ServiceRequest {
                req_id: request.req_id,
                start_at: request.start_at,
                method_name: "get_data".to_string(),
                graph_name: graph_name.to_string(),
            });
            client.get_data(req).await?;

            // self.state()
            // .fanout(request.req_id, request.start_at, Vec::new(), Some(graph_name))
            // .await?;


            // let mean_ms = 40.0;
            // let rate = 1.0 / mean_ms;
            // let latency = sample_exponential(rate);
            // tokio::time::sleep(std::time::Duration::from_millis(latency as u64)).await;
        } else {
            //tokio::time::sleep(std::time::Duration::from_millis(3)).await;
            self.state()
            .fanout(request.req_id, request.start_at, Vec::new(), Some(graph_name))
            .await?;
        }

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
