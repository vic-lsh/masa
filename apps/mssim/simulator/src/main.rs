use std::path::PathBuf;

use anyhow::Result;
use client::cli::CliOptions;
use tokio;

mod client;
mod orchestrator;
mod server;
mod validator;

// Include the generated proto code
pub mod proto {
    tonic::include_proto!("sim");
}

async fn run_from_alibaba_trace(trace_dir: &PathBuf) -> Result<()> {
    let config = sim_config::trace::TraceConfig::from_config_dir(trace_dir)
        .map_err(|e| anyhow::anyhow!("Failed to parse Alibaba input directory: {}", e))?;

    validator::validate_config(&config)?;

    orchestrator::alibaba::launch_simulation_from_yaml(config, trace_dir).await?;

    Ok(())
}

async fn run_as_server(opts: &CliOptions) -> Result<()> {
    // Start servers for receiving input
    let http_port = 8080;
    let grpc_port = 50052;

    // Run both servers concurrently
    let orchestrator_addr = opts.orchestrator.clone();
    let http_handle =
        tokio::spawn(
            async move { server::http::start_http_server(http_port, orchestrator_addr).await },
        );

    let orchestrator_addr = opts.orchestrator.clone();
    let grpc_handle =
        tokio::spawn(
            async move { server::grpc::start_grpc_server(grpc_port, orchestrator_addr).await },
        );

    println!("Input parser service started:");
    println!("  - HTTP server running on port {}", http_port);
    println!("  - gRPC server running on port {}", grpc_port);
    println!("  - Orchestrator service address: {}", opts.orchestrator);

    // Wait for both servers
    tokio::try_join!(async { http_handle.await.unwrap() }, async {
        grpc_handle.await.unwrap()
    })?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    // Parse command line arguments
    let opts = client::cli::parse_cli_args();

    if let Some(path) = opts.alibaba_trace {
        run_from_alibaba_trace(&path).await?;
    } else {
        run_as_server(&opts).await?;
    }

    Ok(())
}
