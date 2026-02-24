use std::env;
use std::net::SocketAddr;

use tonic::async_trait;
use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use crate::register_user::register_user_service_server::{
    RegisterUserService, RegisterUserServiceServer,
};
use crate::register_user::{RegisterUserRequest, RegisterUserResponse};
use crate::user::user_service_client::UserServiceClient;

#[derive(Clone, Debug)]
pub struct Args {
    pub listen_addr: String,
    pub user_service_ip: String,
    pub user_service_port: u16,
    pub user_service_replicas: u8,
}

impl Args {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            listen_addr: env::var("REGISTER_USER_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            user_service_ip: env::var("USER_SERVICE_IP")
                .unwrap_or_else(|_| "user-service".to_string()),
            user_service_port: env::var("USER_SERVICE_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .expect("USER_SERVICE_PORT must be a valid number"),
            user_service_replicas: env::var("USER_SERVICE_REPLICAS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()
                .expect("USER_SERVICE_REPLICAS must be a valid number"),
        })
    }
}

#[derive(Clone)]
pub struct RegisterUserServiceImpl {
    user_service: LoadBalancedChannel,
}

impl RegisterUserServiceImpl {
    pub async fn new(args: &Args) -> Result<Self, Box<dyn std::error::Error>> {
        let user_service = LoadBalancedChannel::new(
            args.user_service_ip.clone(),
            args.user_service_port,
            args.user_service_replicas,
        )
        .await;

        Ok(Self { user_service })
    }
}

#[async_trait]
impl RegisterUserService for RegisterUserServiceImpl {
    async fn register_user(
        &self,
        request: Request<RegisterUserRequest>,
    ) -> Result<Response<RegisterUserResponse>, Status> {
        let req = request.into_inner();
        let mut user_client = UserServiceClient::new(self.user_service.clone());

        user_client
            .register_user(Request::new(crate::user::RegisterUserRequest {
                req_id: req.req_id,
                first_name: req.first_name,
                last_name: req.last_name,
                username: req.username,
                password: req.password,
                carrier: req.carrier,
            }))
            .await?;

        Ok(Response::new(RegisterUserResponse {}))
    }
}

pub async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = args.listen_addr.parse()?;
    let service_impl = RegisterUserServiceImpl::new(&args).await?;

    Server::builder()
        .add_service(RegisterUserServiceServer::new(service_impl))
        .serve_with_masa(addr)
        .await?;
    Ok(())
}
