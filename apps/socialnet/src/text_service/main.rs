use std::net::SocketAddr;
use structopt::StructOpt;
use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use socialnet::text_service::server::TextSvcImpl;
use socialnet::text_service::server::text_svc::text_service::text_service_server::TextServiceServer;
use socialnet::text_service::server::Args;

mod server;

#[derive(StructOpt, Debug, Clone)]
pub struct CLIArgs {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(long, env = "TEXT_SERVICE_LISTEN_ADDR", default_value = "0.0.0.0:8080")]
    pub listen_addr: String,
    
    #[structopt(flatten)]
    pub server_args: Args,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let args = CLIArgs::from_args();
    launch_masa_server!(TextServiceServer, args.policy, build_service, args)
}

async fn build_service(args: CLIArgs) -> Result<(TextSvcImpl, SocketAddr), Box<dyn std::error::Error>> {
    let addr = args.listen_addr.parse()?;
    let service = TextSvcImpl::new(&args.server_args).await?;
    log::info!("Text Service listening on {}", addr);
    Ok((service, addr))
}