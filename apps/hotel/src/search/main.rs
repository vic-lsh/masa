#[path = "../config.rs"]
pub mod config;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use config::HotelConfig;
use hotel::init_logging;
use server::hotel_tonic::search::search_server::SearchServer;
use server::SearchImpl;

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

    let search_addr = "[::0]:8660".parse().expect("Failed to parse address");

    let search = SearchImpl::new().await;
    log::warn!("Server listening on {}...", search_addr);
    Server::builder()
        .add_service(SearchServer::new(search))
        .serve_with_masa(search_addr)
        .await?;

    Ok(())
}
