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
use hotel::init_logging;
use server::hotel_tonic::geo::geo_server::GeoServer;
use server::GeoImpl;

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

    let geo_addr = format!("{}:{}", cfg.geo_ip, cfg.geo_port)
        .parse()
        .expect("Failed to parse address");
    let geo = GeoImpl::new(cfg);
    log::warn!("Server listening on {}...", geo_addr);
    Server::builder()
        .add_service(GeoServer::new(geo))
        .serve_with_masa(geo_addr)
        .await?;

    Ok(())
}
