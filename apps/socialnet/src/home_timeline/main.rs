use structopt::StructOpt;
use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use std::net::SocketAddr;
use deadpool_redis::{Config, Runtime};
use crate::server::home_timeline::home_timeline_service_server::HomeTimelineServiceServer;
use crate::server::{Args, HomeTimelineService};

mod server;

#[derive(StructOpt, Debug, Clone)]
pub struct CLIArgs {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(long, env = "HOME_TIMELINE_LISTEN_ADDR", default_value = "0.0.0.0:8080")]
    pub listen_addr: String,

    #[structopt(long, env = "HOME_TIMELINE_REDIS_URL")]
    pub redis_url: String,

    #[structopt(flatten)]
    pub server_args: Args,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let args = CLIArgs::from_args();
    launch_masa_server!(HomeTimelineServiceServer, args.policy, build_service, args)
}

async fn build_service(args: CLIArgs) -> Result<(HomeTimelineService, SocketAddr), Box<dyn std::error::Error>> {
    let addr = args.listen_addr.parse()?;
    
    let cfg = Config::from_url(args.redis_url);
    let redis_pool = cfg.create_pool(Some(Runtime::Tokio1))?;

    let service = HomeTimelineService::new(redis_pool, &args.server_args).await?;
    log::info!("HomeTimelineService listening on {}", addr);
    Ok((service, addr))
}