#[path = "../config.rs"]
pub mod config;
mod db;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;

use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use config::HotelConfig;
use server::GeoImpl;

use server::hotel_tonic::geo::geo_server::GeoServer;
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
    launch_masa_server!(GeoServer, args.policy, build_service, args)
}

async fn build_service(
    args: Args,
) -> Result<(GeoImpl, SocketAddr), Box<dyn std::error::Error>> {
    let cfg: HotelConfig = {
        let file = File::open(args.config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };

    let HotelConfig { geo, .. } = cfg;

    let geo_addr = format!("{}:{}", "[::]", geo.port)
        .parse()
        .expect("Failed to parse address");

    log::warn!("Server listening on {}...", geo_addr);

    let geo_service = GeoImpl::new(geo);

    Ok((geo_service, geo_addr))
}