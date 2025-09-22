mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use server::synthetic_tonic::child::child_server::ChildServer;
use server::ChildImpl;
use synthetic_app::config::SyntheticConfig;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Synthetic Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    let cfg: SyntheticConfig = {
        let file = File::open(args.config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };

    const PORT: usize = 8000;
    let child_addr = format!("{}:{}", "[::]", PORT)
        .parse()
        .expect("Failed to parse address");
    let child = ChildImpl::new(cfg);
    log::warn!("Server listening on {}...", child_addr);
    Server::builder()
        .add_service(ChildServer::new(child))
        .serve_with_masa(child_addr)
        .await?;

    Ok(())
}
