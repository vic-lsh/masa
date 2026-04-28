use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use synthbench::{ChildImpl, ChildServer, SynthbenchConfig};

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
        serde_json::from_reader(reader).map_err(|e| {
            let path = args.config.display();
            format!("Failed to parse config file '{}': {}", path, e)
        })?
    };

    const PORT: usize = 8000;
    let child_addr = format!("{}:{}", "[::]", PORT)
        .parse()
        .expect("Failed to parse address");
    let child = ChildImpl::new(cfg).await;
    log::warn!("Server listening on {}...", child_addr);
    Server::builder()
        .add_service(ChildServer::new(child))
        .serve(child_addr)
        .await?;

    Ok(())
}
