#[path = "../config.rs"]
pub mod config;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;

use hyper::rt::Exec;
use structopt::StructOpt;
use tonic::{masa::AsyncTaskMetadata, transport::Server};

use config::HotelConfig;
use reboot_hotel::{init_logging, ExecImpl};
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
        cfg.reservation_prob_hotel_avail,
        cfg.payload,
        cfg.reservation_memcached_addr,
        cfg.cache_conn,
        cfg.prob_cache_miss,
        cfg.reservation_mongodb_addr,
    )
    .await?;

    static SMOL_EX: smol::Executor<'static, AsyncTaskMetadata> = smol::Executor::new();
    let ex = Arc::new(ExecImpl::new(&SMOL_EX));
    let ex_clone = ex.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(ex_clone.run());
    });

    let reservation_addr = "[::1]:8665".parse().expect("Failed to parse address");
    log::info!("Server listening on {}...", reservation_addr);
    Server::builder()
        .add_service(ReservationServer::new(reservation))
        .serve_with_executor(reservation_addr, Exec::Executor(ex))
        .await?;

    Ok(())
}
