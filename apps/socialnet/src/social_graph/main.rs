use structopt::StructOpt;
use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use std::net::SocketAddr;
use deadpool_redis::{Config, Runtime};
use crate::server::social_graph::social_graph_service_server::SocialGraphServiceServer;
use crate::server::{Args, SocialGraphService};

mod server;

#[derive(StructOpt, Debug, Clone)]
pub struct CLIArgs {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(long, env = "SOCIAL_GRAPH_LISTEN_ADDR", default_value = "0.0.0.0:8080")]
    pub listen_addr: String,

    #[structopt(long, env = "MONGO_URL")]
    pub mongo_url: String,

    #[structopt(long, env = "REDIS_URL")]
    pub redis_url: String,
    
    #[structopt(flatten)]
    pub server_args: Args,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let args = CLIArgs::from_args();
    launch_masa_server!(SocialGraphServiceServer, args.policy, build_service, args)
}

async fn build_service(args: CLIArgs) -> Result<(SocialGraphService, SocketAddr), Box<dyn std::error::Error>> {
    let addr = args.listen_addr.parse()?;
    
    let cfg = Config::from_url(args.redis_url);
    let redis_pool = cfg.create_pool(Some(Runtime::Tokio1))?;
    
    let service = SocialGraphService::new(&args.mongo_url, redis_pool, &args.server_args).await?;
    log::info!("SocialGraphService listening on {}", addr);
    Ok((service, addr))
}