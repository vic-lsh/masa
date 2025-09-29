#[path = "../config.rs"]
mod config;
mod db;
mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use config::HotelConfig;
use server::hotel_tonic::profile::profile_server::ProfileServer;
use server::ProfileImpl;

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

    let HotelConfig {
        profile, global, ..
    } = cfg;

    let profile_addr = format!("{}:{}", "0.0.0.0", profile.port)
        .parse()
        .expect("Failed to parse address");
    log::warn!("Server listening on {}...", profile_addr);
    let profile_service = ProfileImpl::new(profile, global).await?;
    Server::builder()
        .add_service(ProfileServer::new(profile_service))
        .serve_with_masa(profile_addr)
        .await?;

    Ok(())
}
