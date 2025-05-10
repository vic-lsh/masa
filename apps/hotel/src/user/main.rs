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
use server::hotel::user::user_server::UserServer;
use server::UserImpl;

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

    let user_addr = format!("{}:{}", "[::]", cfg.user_port)
        .parse()
        .expect("Failed to parse address");
    log::warn!("Server listening on {}...", user_addr);
    let user = UserImpl::new(cfg).await?;
    Server::builder()
        .add_service(UserServer::new(user))
        .serve_with_masa(user_addr)
        .await?;

    Ok(())
}
