pub mod server;

use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use reboot_hotel::init_logging;
use server::hotel::frontend::frontend_server::FrontendServer;
use server::FrontendImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

//#[tokio::main(flavor = "current_thread")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let _args = Args::from_args();

    let frontend_addr = "[::1]:8660".parse().expect("Failed to parse address");
    let search_addr = "http://[::1]:8661".to_string();
    let profile_addr = "http://[::1]:8664".to_string();

    let frontend = FrontendImpl::new(search_addr, profile_addr).await;
    log::info!("Server listening on {}...", frontend_addr);
    Server::builder()
        .add_service(FrontendServer::new(frontend))
        .serve_with_masa(frontend_addr)
        .await?;

    Ok(())
}
