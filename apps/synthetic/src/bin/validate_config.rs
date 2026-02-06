use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use structopt::StructOpt;
use synthetic::config::{validate_call_graph, SyntheticConfig};

#[derive(StructOpt, Debug)]
#[structopt(
    name = "validate_config",
    about = "Validate synthetic experiment configuration"
)]
struct Args {
    #[structopt(short, long, parse(from_os_str))]
    config: PathBuf,
}

fn main() {
    let args = Args::from_args();

    // Open file
    let file = match File::open(&args.config) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "Error: Failed to open config file '{}': {}",
                args.config.display(),
                e
            );
            std::process::exit(1);
        }
    };

    let reader = BufReader::new(file);

    // Parse JSON
    let config: SyntheticConfig = match serde_json::from_reader(reader) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: Failed to parse config JSON: {}", e);
            std::process::exit(1);
        }
    };

    // Validate Logic
    if let Err(e) = validate_call_graph(&config.call_graph) {
        eprintln!("Error: Config validation failed: {}", e);
        std::process::exit(1);
    }

    println!("Config is valid.");
}
