#[path = "../config.rs"]
mod config;
mod db;
mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use config::HotelConfig;
use reboot_hotel::init_logging;
use server::hotel::recommendation::recommendation_server::RecommendationServer;
use server::RecommendationImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    let cfg: HotelConfig = {
        let file = File::open(args.config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };
    log::info!("Hotel config: {:?}", cfg);

    let rec = RecommendationImpl::new();

    let rec_addr = "[::1]:8665".parse().expect("Failed to parse address");
    log::info!("Server listening on {}...", rec_addr);
    Server::builder()
        .add_service(RecommendationServer::new(rec))
        .serve_with_masa(rec_addr)
        .await?;

    Ok(())
}
