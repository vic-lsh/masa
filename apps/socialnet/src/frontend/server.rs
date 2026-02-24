use std::env;
use std::net::SocketAddr;

use tonic::async_trait;
use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use crate::compose_post::compose_post_service_client::ComposePostServiceClient;
use crate::compose_post::{ComposePostRequest, ComposePostResponse};
use crate::frontend::frontend_service_server::{FrontendService, FrontendServiceServer};
use crate::frontend::{PingRequest, PingResponse};
use crate::register_user::register_user_service_client::RegisterUserServiceClient;
use crate::register_user::{RegisterUserRequest, RegisterUserResponse};

#[derive(Clone, Debug)]
pub struct Args {
    pub listen_addr: String,
    pub compose_post_ip: String,
    pub compose_post_port: u16,
    pub compose_post_replicas: u8,
    pub register_user_ip: String,
    pub register_user_port: u16,
    pub register_user_replicas: u8,
}

impl Args {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            listen_addr: env::var("FRONTEND_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            compose_post_ip: env::var("COMPOSE_POST_SERVICE_IP")
                .unwrap_or_else(|_| "compose-post-service".to_string()),
            compose_post_port: env::var("COMPOSE_POST_SERVICE_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .expect("COMPOSE_POST_SERVICE_PORT must be a valid number"),
            compose_post_replicas: env::var("COMPOSE_POST_SERVICE_REPLICAS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()
                .expect("COMPOSE_POST_SERVICE_REPLICAS must be a valid number"),
            register_user_ip: env::var("REGISTER_USER_SERVICE_IP")
                .unwrap_or_else(|_| "register-user-service".to_string()),
            register_user_port: env::var("REGISTER_USER_SERVICE_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .expect("REGISTER_USER_SERVICE_PORT must be a valid number"),
            register_user_replicas: env::var("REGISTER_USER_SERVICE_REPLICAS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()
                .expect("REGISTER_USER_SERVICE_REPLICAS must be a valid number"),
        })
    }
}

#[derive(Clone)]
pub struct FrontendServiceImpl {
    compose_post: LoadBalancedChannel,
    register_user: LoadBalancedChannel,
}

impl FrontendServiceImpl {
    pub async fn new(args: &Args) -> Result<Self, Box<dyn std::error::Error>> {
        let compose_post = LoadBalancedChannel::new(
            args.compose_post_ip.clone(),
            args.compose_post_port,
            args.compose_post_replicas,
        )
        .await;
        let register_user = LoadBalancedChannel::new(
            args.register_user_ip.clone(),
            args.register_user_port,
            args.register_user_replicas,
        )
        .await;
        Ok(Self {
            compose_post,
            register_user,
        })
    }
}

#[async_trait]
impl FrontendService for FrontendServiceImpl {
    async fn ping(&self, request: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        let msg = request.into_inner().message;
        Ok(Response::new(PingResponse { message: msg }))
    }

    async fn compose_post(
        &self,
        request: Request<ComposePostRequest>,
    ) -> Result<Response<ComposePostResponse>, Status> {
        let mut client = ComposePostServiceClient::new(self.compose_post.clone());
        client.compose_post(request).await
    }

    async fn register_user(
        &self,
        request: Request<RegisterUserRequest>,
    ) -> Result<Response<RegisterUserResponse>, Status> {
        let mut client = RegisterUserServiceClient::new(self.register_user.clone());
        client.register_user(request).await
    }
}

pub async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = args.listen_addr.parse()?;
    let service_impl = FrontendServiceImpl::new(&args).await?;

    Server::builder()
        .add_service(FrontendServiceServer::new(service_impl))
        .serve_with_masa(addr)
        .await?;
    Ok(())
}
