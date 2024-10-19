#[path = "../config.rs"]
pub mod config;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use config::HotelConfig;
use reboot_hotel::init_logging;
use server::hotel::geo::geo_server::GeoServer;
use server::GeoImpl;

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

    let geo = GeoImpl::new(cfg.hotels, cfg.geo_range);

    let geo_addr = "[::1]:8662".parse().expect("Failed to parse address");
    log::info!("Server listening on {}...", geo_addr);
    Server::builder()
        .add_service(GeoServer::new(geo))
        .serve_with_masa(geo_addr)
        .await?;

    Ok(())
}
