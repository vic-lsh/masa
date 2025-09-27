#[path = "../config.rs"]
pub mod config;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use config::HotelConfig;
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

    let HotelConfig {
        search, geo, rate, ..
    } = cfg;

    let search_addr = format!("{}:{}", "[::]", search.port)
        .parse()
        .expect("Failed to parse address");

    log::warn!("Server listening on {}...", search_addr);
    let search_service = SearchImpl::new(geo, rate).await;
    Server::builder()
        .add_service(SearchServer::new(search_service))
        .serve_with_masa(search_addr)
        .await?;

    Ok(())
}
