pub mod server;

use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use reboot_hotel::init_logging;
use server::hotel::search::search_server::SearchServer;
use server::SearchImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let _args = Args::from_args();

    let search_addr = "[::1]:8661".parse().expect("Failed to parse address");
    let geo_addr = "http://[::1]:8662".to_string();
    let rate_addr = "http://[::1]:8663".to_string();

    let search = SearchImpl::new(geo_addr, rate_addr).await;
    log::info!("Server listening on {}...", search_addr);
    Server::builder()
        .add_service(SearchServer::new(search))
        .serve_with_masa(search_addr)
        .await?;

    Ok(())
}
