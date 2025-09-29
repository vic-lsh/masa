#[path = "../config.rs"]
pub mod config;
mod db;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use config::HotelConfig;
use server::hotel_tonic::reservation::reservation_server::ReservationServer;
use server::ReservationImpl;

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

    let HotelConfig { reservation, .. } = cfg;

    let reservation_addr = format!("{}:{}", "0.0.0.0", reservation.port)
        .parse()
        .expect("Failed to parse address");
    log::warn!("Server listening on {}...", reservation_addr);
    let reservation_service = ReservationImpl::new(reservation).await?;
    Server::builder()
        .add_service(ReservationServer::new(reservation_service))
        .serve_with_masa(reservation_addr)
        .await?;

    Ok(())
}
