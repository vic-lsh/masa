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
use server::hotel::rate::rate_server::RateServer;
use server::RateImpl;

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

    let rate = RateImpl::new(
        cfg.hotels,
        cfg.payload,
        cfg.rate_memcached_addr,
        cfg.cache_conn,
        cfg.cache_miss_rate,
        cfg.rate_mongodb_addr,
    )
    .await?;

    let rate_addr = "[::1]:8663".parse().expect("Failed to parse address");
    log::info!("Server listening on {}...", rate_addr);
    Server::builder()
        .add_service(RateServer::new(rate))
        .serve_with_masa(rate_addr)
        .await?;

    Ok(())
}
