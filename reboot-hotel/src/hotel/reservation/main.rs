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
use server::hotel::reservation::reservation_server::ReservationServer;
use server::ReservationImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    let cfg: HotelConfig = {
        let file = File::open(args.config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };
    log::info!("Hotel config: {:?}", cfg);

    let reservation = ReservationImpl::new(
        cfg.hotels,
        cfg.payload,
        cfg.reservation_memcached_addr,
        cfg.cache_conn,
        cfg.cache_miss_rate,
        cfg.reservation_mongodb_addr,
    )
    .await?;

    let reservation_addr = "[::1]:8665".parse().expect("Failed to parse address");
    log::info!("Server listening on {}...", reservation_addr);
    Server::builder()
        .add_service(ReservationServer::new(reservation))
        .serve_with_masa(reservation_addr)
        .await?;

    Ok(())
}
