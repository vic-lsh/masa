#[path = "../config.rs"]
pub mod config;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;

use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use config::HotelConfig;
use server::FrontendImpl;

use server::hotel_tonic::frontend::frontend_server::FrontendServer;
use std::net::SocketAddr;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let args = Args::from_args();
    launch_masa_server!(FrontendServer, args.policy, build_service, args)
}

async fn build_service(
    args: Args,
) -> Result<(FrontendImpl, SocketAddr), Box<dyn std::error::Error>> {
    let cfg: HotelConfig = {
        let file = File::open(args.config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };

    let frontend_addr = format!("{}:{}", "[::]", cfg.frontend.port)
        .parse()
        .expect("Failed to parse address");

    let frontend_service = FrontendImpl::new(cfg).await;

    log::info!("Server listening on {}...", frontend_addr);

    Ok((frontend_service, frontend_addr))
}