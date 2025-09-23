use anyhow::{Context, Result};
use futures::future;
use prost_types::Timestamp;
use rand_distr::{Bernoulli, Distribution, Normal};
use serde::{Deserialize, Serialize};
use service_stubs::service_client::ServiceClient;
use sim_config::deployment::Deployment;
use sim_config::svc::{ServiceName, ServiceTraceConfig};
use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tonic::transport::Channel;
use tonic::{transport::Server, Request, Response, Status};
use tracing::{error, warn};

pub mod service_stubs {
    tonic::include_proto!("service");
}

use service_stubs::service_server::{Service, ServiceServer};
use service_stubs::{CallData, RootRequest, RootResponse, ServiceRequest, ServiceResponse};

#[derive(Serialize, Deserialize)]
struct ServiceConfigFromJSON {
    ip: String,
    port: String,
    methods: HashMap<String, MethodConfigFromJSON>,
}
#[derive(Serialize, Deserialize)]
struct MethodConfigFromJSON {
    calls: Option<Vec<Vec<String>>>,
    latency_distribution: DistributionConfigFromJSON,
    error_rate: DistributionConfigFromJSON,
}

struct ServiceConfig {
    methods: HashMap<String, MethodConfig>,
}

struct MethodConfig {
    calls: Option<Vec<Vec<Call>>>,
    latency_distribution: Box<dyn DistributionSimulator<f64>>,
    error_rate: Box<dyn DistributionSimulator<bool>>,
}

struct Call {
    service_name: String,
    method_name: String,
}

#[derive(Serialize, Deserialize)]
struct DistributionConfigFromJSON {
    distribution_type: String,
    parameters: HashMap<String, f64>,
}

trait DistributionSimulator<T>: Send + Sync {
    fn simulate(&self) -> T;
}

struct NormalDistribution {
    distribution: rand_distr::Normal<f64>,
}

impl DistributionSimulator<f64> for NormalDistribution {
    fn simulate(&self) -> f64 {
        let mut rng = rand::rng();
        self.distribution.sample(&mut rng)
    }
}

struct BernoulliDistribution {
    distribution: rand_distr::Bernoulli,
}

impl DistributionSimulator<bool> for BernoulliDistribution {
    fn simulate(&self) -> bool {
        let mut rng = rand::rng();
        self.distribution.sample(&mut rng)
    }
}

pub struct GenericService {
    config: ServiceConfig,
    config_json: HashMap<String, ServiceConfigFromJSON>,
    services: Arc<Mutex<HashMap<String, ServiceClient<Channel>>>>,
}

#[derive(Clone)]
pub struct ServiceResponseWrapper {
    res: ServiceResponse,
    sent_at: Timestamp,
    received_at: Timestamp,
}

#[derive(Clone)]
pub struct ServiceErrorWrapper {
    method_name: String,
    sent_at: Timestamp,
    received_at: Timestamp,
}

impl GenericService {
    pub async fn new() -> Self {
        let config_path_str =
            env::var("CONFIG_PATH").unwrap_or_else(|_| "config/config.json".to_string());
        let config_path = Path::new(&config_path_str);
        let config_json: HashMap<String, ServiceConfigFromJSON> = serde_json::from_str(
            &std::fs::read_to_string(config_path).expect("Failed to read config file"),
        )
        .expect("Failed to parse config file");
        let service_name = env::var("SERVICE_NAME").expect("Failed to get SERVICE_NAME");
        config_json
            .get(&service_name)
            .expect("Own service not found in config");
        let config = ServiceConfig {
            methods: config_json[&service_name]
                .methods
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        MethodConfig {
                            calls: v.calls.as_ref().map(|calls| {
                                calls
                                    .iter()
                                    .map(|call_row| {
                                        call_row
                                            .iter()
                                            .map(|call| {
                                                let mut call_parts = call.split(".");
                                                let service_name =
                                                    call_parts.next().unwrap().to_string();
                                                let method_name =
                                                    call_parts.next().unwrap().to_string();
                                                Call {
                                                    service_name,
                                                    method_name,
                                                }
                                            })
                                            .collect()
                                    })
                                    .collect()
                            }),
                            latency_distribution: match v
                                .latency_distribution
                                .distribution_type
                                .as_str()
                            {
                                "normal" => Box::new(NormalDistribution {
                                    distribution: Normal::new(
                                        v.latency_distribution.parameters["mean"],
                                        v.latency_distribution.parameters["stddev"],
                                    )
                                    .unwrap(),
                                }),
                                _ => panic!("Unsupported distribution type"),
                            },
                            error_rate: match v.error_rate.distribution_type.as_str() {
                                "bernoulli" => Box::new(BernoulliDistribution {
                                    distribution: Bernoulli::new(v.error_rate.parameters["p"])
                                        .unwrap(),
                                }),
                                _ => panic!("Unsupported distribution type"),
                            },
                        },
                    )
                })
                .collect(),
        };
        GenericService {
            config,
            config_json,
            services: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn init_service_client(
        &self,
        service_name: &str,
    ) -> Result<ServiceClient<Channel>, Box<dyn std::error::Error>> {
        if self.services.lock().await.contains_key(service_name) {
            return Ok(self
                .services
                .lock()
                .await
                .get(service_name)
                .unwrap()
                .clone());
        }
        let service_ip = self.config_json[service_name].ip.clone();
        let service_port = self.config_json[service_name].port.clone();
        let service_url = format!("http://{}:{}", service_ip, service_port);
        println!("Connecting to service {} at {}", service_name, service_url);
        let client = ServiceClient::connect(service_url).await?;
        self.services
            .lock()
            .await
            .insert(service_name.to_string(), client.clone());
        Ok(client)
    }

    pub async fn call_service(
        &self,
        service_name: &str,
        method_name: &str,
    ) -> Result<ServiceResponseWrapper, ServiceErrorWrapper> {
        println!(
            "Calling service {} with method {}",
            service_name, method_name
        );

        let mut client = self
            .init_service_client(service_name)
            .await
            .expect("Client connection failed");
        let request = tonic::Request::new(ServiceRequest {
            method_name: method_name.to_string(),
        });
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX EPOCH");
        let sent_at = Timestamp {
            seconds: now.as_secs() as i64,
            nanos: now.subsec_nanos() as i32,
        };
        let response = client.get_data(request).await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX EPOCH");
        let received_at = Timestamp {
            seconds: now.as_secs() as i64,
            nanos: now.subsec_nanos() as i32,
        };
        match response {
            Ok(res) => {
                println!("Response: {:?}", res);
                let srw = ServiceResponseWrapper {
                    res: res.into_inner(),
                    sent_at,
                    received_at,
                };
                Result::Ok(srw)
            }
            Err(e) => {
                eprintln!("Error calling service: {:?}", e);
                Result::Err(ServiceErrorWrapper {
                    method_name: method_name.to_string(),
                    sent_at,
                    received_at,
                })
            }
        }
    }
}

#[tonic::async_trait]
impl Service for GenericService {
    async fn get_data(
        &self,
        request: Request<ServiceRequest>,
    ) -> Result<Response<ServiceResponse>, Status> {
        let method_name = request.into_inner().method_name;
        println!("Received request for method: {}", method_name);
        let method_cnf = self
            .config
            .methods
            .get(&method_name)
            .expect("Method not found in config");
        println!("Simulating Latency");
        // wait latency
        let latency = method_cnf.latency_distribution.simulate();
        sleep(std::time::Duration::from_millis(latency.round() as u64)).await;
        let error_rate = method_cnf.error_rate.simulate();
        if error_rate {
            println!("Simulating Error");
            return Err(Status::internal("Simulated Error"));
        }
        println!("Did not Error");
        let mut call_list = Vec::new();
        match &method_cnf.calls {
            Some(calls) => {
                for call_row in calls {
                    let mut succeeded = vec![false; call_row.len()];
                    while succeeded.contains(&false) {
                        let mut futures = Vec::new();
                        for (i, call) in call_row.iter().enumerate() {
                            if succeeded[i] {
                                continue;
                            }
                            let service_to_call = &call.service_name;
                            let method_to_call = &call.method_name;
                            futures.push(self.call_service(service_to_call, method_to_call));
                        }
                        let resp = future::join_all(futures).await;
                        let mut j = 0;
                        (0..succeeded.len()).for_each(|i| {
                            if !succeeded[i] {
                                succeeded[i] = resp[j].is_ok();
                                let respj = resp[j].clone();
                                let (method_name, sent_at, received_at, was_error) = match &respj {
                                    Ok(r) => {
                                        for c in &r.res.calls {
                                            call_list.push(c.clone());
                                        }
                                        (
                                            r.res.method_name.clone(),
                                            r.sent_at.clone(),
                                            r.received_at.clone(),
                                            false,
                                        )
                                    }
                                    Err(r) => (
                                        r.method_name.clone(),
                                        r.sent_at.clone(),
                                        r.received_at.clone(),
                                        true,
                                    ),
                                };
                                call_list.push(CallData {
                                    method_name,
                                    request_sent_at: Some(sent_at),
                                    response_received_at: Some(received_at),
                                    was_an_error: was_error,
                                });
                                j += 1;
                            }
                        });
                    }
                }
            }
            None => {
                println!("No calls to make");
            }
        }
        Ok(Response::new(ServiceResponse {
            calls: call_list,
            method_name,
        }))
    }

    async fn root(&self, _request: Request<RootRequest>) -> Result<Response<RootResponse>, Status> {
        unimplemented!()
    }
}

#[allow(dead_code)]
struct AlibabaService {
    config: ServiceTraceConfig,
    clients: HashMap<ServiceName, ServiceClient<Channel>>,
    deployment: Deployment,
    self_svc_name: ServiceName,
}

impl AlibabaService {
    pub async fn new(
        self_svc_name: ServiceName,
        config: ServiceTraceConfig,
        deployment: Deployment,
    ) -> Result<Self> {
        let children = config.call_graph.callees_of(&self_svc_name);

        let clients = Self::connect_to_children(&children, &deployment).await?;

        Ok(AlibabaService {
            config,
            clients,
            deployment,
            self_svc_name,
        })
    }

    async fn connect_to_children(
        children: &[ServiceName],
        deployment: &Deployment,
    ) -> Result<HashMap<ServiceName, ServiceClient<Channel>>> {
        let mut clients = HashMap::new();
        for child_svc_name in children {
            let svc = deployment.services.get(child_svc_name).with_context(|| {
                format!("Child service {} not found in deployment", child_svc_name)
            })?;
            let addr = format!("http://{}:{}", svc.ip, svc.port);
            let client = ServiceClient::connect(addr).await?;
            clients.insert(child_svc_name.clone(), client);
        }
        Ok(clients)
    }
}

#[tonic::async_trait]
impl Service for AlibabaService {
    async fn get_data(
        &self,
        request: Request<ServiceRequest>,
    ) -> Result<Response<ServiceResponse>, Status> {
        let method_name = request.into_inner().method_name;

        self.handle_method(method_name.clone()).await?;

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

        // All the root service does is calling into internal services
        self.fanout().await?;

        Ok(Response::new(RootResponse {}))
    }
}

fn busy_spin(duration_ms: f64) {
    let start = std::time::Instant::now();
    let duration = std::time::Duration::from_millis(duration_ms as u64);
    while std::time::Instant::now() - start < duration {
        // Busy spin
    }
}

impl AlibabaService {
    async fn handle_method(&self, method_name: String) -> Result<(), Status> {
        let name = method_name.into();
        let latency_dist = self
            .config
            .method_latency
            .get_method_dist(&name)
            .ok_or(Status::not_found("Method not found"))?;

        let total_latency_ms = latency_dist.sample(&mut rand::rng());

        let start = Instant::now();
        self.fanout().await?;
        let elapsed = start.elapsed();

        let remaining = total_latency_ms - (elapsed.as_millis() as f64);
        if remaining > 0.0 {
            busy_spin(remaining);
        } else {
            warn!(
                "Warning: fanout took longer ({:?}) than total latency ({:.2} ms)",
                elapsed, total_latency_ms
            );
        }

        Ok(())
    }

    async fn fanout(&self) -> Result<(), Status> {
        let mut tasks = Vec::new();
        for (child_svc_name, client) in &self.clients {
            let method_to_call = self
                .config
                .method_freq_map
                .get_service(child_svc_name)
                .and_then(|sampler| Some(sampler.sample(&mut rand::rng()).to_string()))
                .ok_or(Status::not_found(format!(
                    "Configuration error: Service {child_svc_name} has no method to call"
                )))?;

            let mut client = client.clone();
            let request = tonic::Request::new(ServiceRequest {
                method_name: method_to_call,
            });

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let deployment_path_str =
        env::var("DEPLOYMENT_CONFIG_PATH").unwrap_or_else(|_| "config/deployment.json".to_string());
    let config_dir_str = env::var("CONFIG_PATH").unwrap_or_else(|_| "config/".to_string());
    let service_name = env::var("SERVICE_NAME").expect("Failed to get SERVICE_NAME");
    let port = env::var("SERVICE_PORT").unwrap_or_else(|_| "50051".to_string());

    let path = config_dir_str.into();
    let config = ServiceTraceConfig::from_config_dir(&path, &service_name)
        .expect("Loading config should succeed");

    let deployment_path = deployment_path_str.into();
    let deployment = Deployment::read_from_file(&deployment_path)?;

    let svc =
        AlibabaService::new(ServiceName::from_string(service_name), config, deployment).await?;

    let addr = format!("0.0.0.0:{}", port).parse()?;
    println!("🚀 Generic Service listening on {}", addr);

    Server::builder()
        .add_service(ServiceServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}
