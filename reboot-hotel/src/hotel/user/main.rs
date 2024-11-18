#[path = "../config.rs"]
pub mod config;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use config::HotelConfig;
use reboot_hotel::init_logging;
use server::hotel::user::user_server::UserServer;
use server::UserImpl;

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
    log::warn!("Hotel config: {:?}", cfg);

    let user = UserImpl::new(
        cfg.user_users,
        cfg.user_mongodb_addr,
        cfg.user_prob_check_user,
    )
    .await?;

    let user_addr = "[::1]:8666".parse().expect("Failed to parse address");
    log::warn!("Server listening on {}...", user_addr);
    Server::builder()
        .add_service(UserServer::new(user))
        .serve_with_masa(user_addr)
        .await?;

    Ok(())
}
