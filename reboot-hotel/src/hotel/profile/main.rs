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
use server::hotel::profile::profile_server::ProfileServer;
use server::ProfileImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

// #[tokio::main(flavor = "current_thread")]
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

    let profile = ProfileImpl::new(
        cfg.hotels,
        cfg.payload,
        cfg.profile_memcached_addr,
        cfg.cache_conns,
        cfg.prob_cache_miss,
        cfg.profile_mongodb_addr,
    )
    .await?;

    let profile_addr = "[::0]:8660".parse().expect("Failed to parse address");
    log::warn!("Server listening on {}...", profile_addr);
    Server::builder()
        .add_service(ProfileServer::new(profile))
        .serve_with_masa(profile_addr)
        .await?;

    Ok(())
}
