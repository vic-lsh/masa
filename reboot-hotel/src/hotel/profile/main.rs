use config::Config;
use server::masa::profile::profile_server::ProfileServer;
use server::ProfileImpl;
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

    let profile_addr = format!("[::]:{}", cfg.profile_port)
        .parse()
        .expect("Failed to parse address");

    let profile = ProfileImpl::new(
        cfg.hotels,
        cfg.payload,
        cfg.profile_memcached_addr,
        cfg.cache_conn,
        cfg.cache_miss_rate,
        cfg.profile_mongodb_addr,
    )
    .await?;
    eprintln!("Server listening on {}...", profile_addr);
    Server::builder()
        .add_service(ProfileServer::new(profile))
        .serve(profile_addr)
        .await?;

    Ok(())
}
