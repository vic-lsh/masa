use config::Config;
use server::masa::rate::rate_server::RateServer;
use server::RateImpl;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use structopt::StructOpt;
use tonic::transport::Server;

#[path = "../config.rs"]
pub mod config;
pub mod server;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
    // #[structopt(short, long, required = true)]
    // pub output: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();
    let file = File::open(args.config).expect("Failed to open file");
    let reader = BufReader::new(file);
    let cfg: Config = serde_json::from_reader(reader)?;
    eprintln!("Hotel config: {:?}", cfg);

    let rate_addr = format!("[::]:{}", cfg.rate_port)
        .parse()
        .expect("Failed to parse address");

    let rate = RateImpl::new(
        cfg.hotels,
        cfg.payload,
        cfg.rate_memcached_addr,
        cfg.cache_conn,
        cfg.cache_miss_rate,
        cfg.rate_mongodb_addr,
    )
    .await?;
    eprintln!("Server listening on {}...", rate_addr);
    Server::builder()
        .add_service(RateServer::new(rate))
        .serve(rate_addr)
        .await?;

    Ok(())
}
