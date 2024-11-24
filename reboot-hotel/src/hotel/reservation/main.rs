#[path = "../config.rs"]
pub mod config;
mod db;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use config::HotelConfig;
use reboot_hotel::init_logging;
use server::hotel::reservation::reservation_server::ReservationServer;
use server::ReservationImpl;

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
    log::warn!("Hotel config: {:?}", cfg);

    let reservation = ReservationImpl::new(
        cfg.hotels,
        cfg.reservation_dates,
        cfg.reservation_prob_hotel_avail,
        cfg.reservation_memcached_addr,
        cfg.cache_conns,
        cfg.prob_cache_miss,
        cfg.reservation_mongodb_addr,
    )
    .await?;

    let reservation_addr = "[::0]:8660".parse().expect("Failed to parse address");
    log::warn!("Server listening on {}...", reservation_addr);
    Server::builder()
        .add_service(ReservationServer::new(reservation))
        .serve_with_masa(reservation_addr)
        .await?;

    Ok(())
}
