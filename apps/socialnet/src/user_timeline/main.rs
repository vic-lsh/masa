use structopt::StructOpt;
use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use std::net::SocketAddr;
use crate::server::user_timeline::user_timeline_service_server::UserTimelineServiceServer;
use crate::server::{Args, UserTimelineServiceImpl};

mod server;

#[derive(StructOpt, Debug, Clone)]
pub struct CLIArgs {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(flatten)]
    pub server_args: Args,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let args = CLIArgs::from_args();
    launch_masa_server!(UserTimelineServiceServer, args.policy, build_service, args)
}

async fn build_service(args: CLIArgs) -> Result<(UserTimelineServiceImpl, SocketAddr), Box<dyn std::error::Error>> {
    let addr = args.server_args.listen_addr.parse()?;
    let service = UserTimelineServiceImpl::new(args.server_args).await?;
    log::info!("UserTimelineService listening on {}", addr);
    Ok((service, addr))
}