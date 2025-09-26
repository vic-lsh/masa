use std::path::PathBuf;
use structopt::StructOpt;

use crate::orchestrator::Backend;

#[derive(Debug, StructOpt)]
#[structopt(
    name = "microservice-simulator-parser",
    about = "Microservice Simulator Input Parser"
)]
pub struct CliOptions {
    #[structopt(short, long, parse(from_os_str))]
    /// Path to the input JSON file
    pub alibaba_trace: Option<PathBuf>,

    #[structopt(long, parse(from_os_str))]
    /// Override the replay trace file used by the load generator
    pub replay_path: Option<PathBuf>,

    #[structopt(short, long, parse(from_os_str))]
    /// Path to the input JSON file
    pub config_dir: PathBuf,

    #[structopt(short, long, default_value = "localhost:50051")]
    /// Address of the orchestrator service
    pub orchestrator: String,

    #[structopt(
        long = "orchestrator-backend",
        default_value = "docker-compose",
        possible_values = Backend::variants(),
        case_insensitive = true
    )]
    /// Execution backend used to orchestrate the simulation (docker-compose | k8s)
    pub orchestrator_backend: Backend,
}

pub fn parse_cli_args() -> CliOptions {
    CliOptions::from_args()
}
