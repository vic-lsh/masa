use std::path::PathBuf;
use structopt::StructOpt;

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
    pub config_dir: Option<PathBuf>,

    #[structopt(long, parse(from_os_str), default_value = "./docker-compose.yml")]
    /// Path to the generated docker compose file
    pub docker_compose_output_path: PathBuf,

    #[structopt(
        long,
        parse(from_os_str),
        default_value = "./service_configs/deployment.json"
    )]
    /// Path to the generated deployment file
    pub deployment_output_path: PathBuf,

    #[structopt(short, long, default_value = "localhost:50051")]
    /// Address of the orchestrator service
    pub orchestrator: String,
}

pub fn parse_cli_args() -> CliOptions {
    CliOptions::from_args()
}
