use std::env;
use std::net::SocketAddr;
use structopt::StructOpt;
use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use crate::media::media_service_server::MediaServiceServer;
use crate::server::MediaServiceImpl;

mod server;

pub mod media {
    tonic::include_proto!("media");
}

#[derive(StructOpt, Debug, Clone)]
pub struct Args {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(long, env = "MEDIA_SERVICE_LISTEN_ADDR", default_value = "0.0.0.0:8080")]
    pub listen_addr: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let args = Args::from_args();
    launch_masa_server!(MediaServiceServer, args.policy, build_service, args)
}

async fn build_service(
    args: Args,
) -> Result<(MediaServiceImpl, SocketAddr), Box<dyn std::error::Error>> {
    let addr = args.listen_addr.parse()?;
    log::info!("Media Service listening on {}", addr);
    let service = MediaServiceImpl::new();
    Ok((service, addr))
}