mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use server::synthetic_tonic::child::child_server::ChildServer;
use server::ChildImpl;
use synthetic::config::SyntheticConfig;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Synthetic Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    log::info!("Scheduler mode: {:?}", tokio::runtime::get_sched_flavor());

    let args = Args::from_args();
    let cfg: SyntheticConfig = {
        let file = File::open(&args.config)
            .unwrap_or_else(|e| panic!("Failed to open file '{}': {}", args.config.display(), e));
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).map_err(|e| {
            let path = args.config.display();
            format!("Failed to parse config file '{}': {}", path, e)
        })?
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
