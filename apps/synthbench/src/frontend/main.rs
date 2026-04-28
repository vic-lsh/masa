use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use synthbench::{FrontendImpl, FrontendServer, SynthbenchConfig};

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Synthbench Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[cfg(not(feature = "sched_mt"))]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    main_inner().await
}

#[cfg(feature = "sched_mt")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workers = std::env::var("CPUS_PER_REPLICA")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|f| (f as usize).max(1))
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        });
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?
        .block_on(main_inner())
}

async fn main_inner() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    log::info!("Scheduler mode: {:?}", tokio::runtime::get_sched_flavor());

    let args = Args::from_args();
    let cfg: SynthbenchConfig = {
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
        .serve(frontend_addr)
        .await?;

    Ok(())
}
