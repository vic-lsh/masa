use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use synthetic::{SyntheticConfig, FrontendServer, FrontendImpl};

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
        serde_json::from_reader(reader)
            .map_err(|e| format!("Failed to parse config '{}': {}", args.config.display(), e))?
    };

    const PORT: u16 = 8000;
    let frontend_addr = format!("{}:{}", "[::]", PORT)
        .parse()
        .expect("Failed to parse address");
    let frontend = FrontendImpl::new(cfg).await;
    log::info!("Server listening on {}...", frontend_addr);
    Server::builder()
        .add_service(FrontendServer::new(frontend))
        .serve_with_masa(frontend_addr)
        .await?;

    Ok(())
}
