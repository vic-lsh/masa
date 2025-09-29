#[path = "../config.rs"]
mod config;
mod db;
mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use config::HotelConfig;
use server::hotel_tonic::rate::rate_server::RateServer;
use server::RateImpl;

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

    let HotelConfig { rate, global, .. } = cfg;

    let rate_addr = format!("{}:{}", "0.0.0.0", rate.port)
        .parse()
        .expect("Failed to parse address");
    let rate_service = RateImpl::new(rate, global).await?;
    log::warn!("Server listening on {}...", rate_addr);
    Server::builder()
        .add_service(RateServer::new(rate_service))
        .serve_with_masa(rate_addr)
        .await?;

    Ok(())
}
