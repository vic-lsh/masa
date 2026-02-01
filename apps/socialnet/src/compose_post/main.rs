use socialnet::compose_post::server::{Args, ComposePostServiceImpl};
use structopt::StructOpt;
use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use std::net::SocketAddr;
use socialnet::compose_post::compose_post_service_server::ComposePostServiceServer;

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
    launch_masa_server!(ComposePostServiceServer, args.policy, build_service, args)
}

async fn build_service(
    args: CLIArgs,
) -> Result<(ComposePostServiceImpl, SocketAddr), Box<dyn std::error::Error>> {
    let addr = args.server_args.listen_addr.parse()?;
    let service_impl = ComposePostServiceImpl::new(&args.server_args).await?;
    log::info!("ComposePost service listening on {}", addr);
    Ok((service_impl, addr))
}