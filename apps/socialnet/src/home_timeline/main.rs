use crate::server::home_timeline::home_timeline_service_server::HomeTimelineServiceServer;
use clap::Parser;
use log::info;
use tonic::transport::Server;

mod server;
use server::HomeTimelineService;

/// The command-line arguments for the home timeline service.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The IP address and port to bind the server to.
    #[arg(short, long, default_value = "0.0.0.0:8080")]
    addr: String,

    /// The connection URL for the Redis primary instance.
    // #[arg(long, env = "REDIS_PRIMARY_URL")]
    // redis_primary_url: String,
    #[arg(long, env = "REDIS_URL")]
    redis_primary_url: String,

    /// The connection URL for the Redis replica instance (optional).
    /// Used for read operations if provided.
    #[arg(long, env = "REDIS_REPLICA_URL")]
    redis_replica_url: Option<String>,

    /// The gRPC endpoint for the Post Storage service.
    #[arg(long, env = "POST_STORAGE_SERVICE_ADDR")]
    post_storage_service_addr: String,

    /// The gRPC endpoint for the Social Graph service.
    #[arg(long, env = "SOCIAL_GRAPH_SERVICE_ADDR")]
    social_graph_service_addr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    let service = HomeTimelineService::new(
        &args.redis_primary_url,
        args.redis_replica_url.as_deref(),
        &args.post_storage_service_addr,
        &args.social_graph_service_addr,
    )
    .await?;

    let addr = args.addr.parse()?;
    info!("HomeTimelineService listening on {}", addr);

    Server::builder()
        .add_service(HomeTimelineServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
