use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use structopt::StructOpt;

use app_utils::logging::init_logging;
use app_utils::{config::PolicyArgs, launch_masa_server};
use synthetic::{ChildImpl, ChildServer, SyntheticConfig};
use std::net::SocketAddr;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Synthetic Args")]
pub struct Args {
    #[structopt(flatten)]
    pub policy: PolicyArgs,

    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let args = Args::from_args();
    launch_masa_server!(ChildServer, args.policy, build_service, args)
}

async fn build_service(
    args: Args,
) -> Result<(ChildImpl, SocketAddr), Box<dyn std::error::Error>> {
    log::info!("Scheduler mode: {:?}", tokio::runtime::get_sched_flavor());

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
    let child = ChildImpl::new(cfg).await;
    log::warn!("Server listening on {}...", child_addr);
    
    Ok((child, child_addr))
}