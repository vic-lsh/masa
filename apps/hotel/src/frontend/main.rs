#[path = "../config.rs"]
pub mod config;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use config::HotelConfig;
use hotel::init_logging;
use server::hotel_tonic::frontend::frontend_server::FrontendServer;
use server::FrontendImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[tokio::main(flavor = "current_thread")]
//#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    let cfg: HotelConfig = {
        let file = File::open(args.config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };
    log::warn!("Hotel config: {:?}", cfg);

    let frontend_addr = "[::0]:8660".parse().expect("Failed to parse address");
    let frontend = FrontendImpl::new().await;
    log::info!("Server listening on {}...", frontend_addr);
    Server::builder()
        .add_service(FrontendServer::new(frontend))
        .serve_with_masa(frontend_addr)
        .await?;

    Ok(())
}
