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

    #[structopt(short, long, parse(from_os_str))]
    /// Path to the input JSON file
    pub config_dir: PathBuf,

    #[structopt(short, long, default_value = "localhost:50051")]
    /// Address of the orchestrator service
    pub orchestrator: String,
}

pub fn parse_cli_args() -> CliOptions {
    CliOptions::from_args()
}
