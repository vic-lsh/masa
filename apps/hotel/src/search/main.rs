#[path = "../config.rs"]
pub mod config;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;

use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use config::HotelConfig;
use server::SearchImpl;

use server::hotel_tonic::search::search_server::SearchServer;
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
    launch_masa_server!(SearchServer, args.policy, build_service, args)
}

async fn build_service(
    args: Args,
) -> Result<(SearchImpl, SocketAddr), Box<dyn std::error::Error>> {
    let cfg: HotelConfig = {
        let file = File::open(args.config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };

    let HotelConfig {
        search, geo, rate, ..
    } = cfg;

    let search_addr = format!("{}:{}", "[::]", search.port)
        .parse()
        .expect("Failed to parse address");

    log::warn!("Server listening on {}...", search_addr);

    let search_service = SearchImpl::new(geo, rate).await;

    Ok((search_service, search_addr))
}