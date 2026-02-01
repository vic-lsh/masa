use structopt::StructOpt;
use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use std::net::SocketAddr;
use socialnet::post_storage::post_storage_service_server::PostStorageServiceServer;
use socialnet::post_storage::server::{Args, PostStorageServiceImpl};

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
    launch_masa_server!(PostStorageServiceServer, args.policy, build_service, args)
}

async fn build_service(args: CLIArgs) -> Result<(PostStorageServiceImpl, SocketAddr), Box<dyn std::error::Error>> {
    let addr = args.server_args.listen_addr.parse()?;
    let service = PostStorageServiceImpl::new(&args.server_args).await?;
    log::info!("PostStorageService listening on {}", addr);
    Ok((service, addr))
}