use std::{fs::File, io::BufReader, path::PathBuf};

use config::HotelConfig;
use app_utils::logging::init_logging;
use server::{hotel_tonic::review::review_server::ReviewServer, ReviewImpl};
use structopt::StructOpt;
use tonic::transport::Server;

#[path = "../config.rs"]
mod config;
mod db;
mod server;

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
    let review_addr = format!("{}:{}", "[::]", cfg.review_port)
        .parse()
        .expect("Failed to parse address");
    let review = ReviewImpl::new(cfg).await?;
    log::warn!("Server listening on {}...", review_addr);
    Server::builder()
        .add_service(ReviewServer::new(review))
        .serve_with_masa(review_addr)
        .await?;

    Ok(())
}
