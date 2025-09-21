use crate::server::social_graph::social_graph_service_server::SocialGraphServiceServer;
use clap::Parser;
use log::info;
use tonic::transport::Server;

mod server;
use server::SocialGraphService;

/// The command-line arguments for the social graph service.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The IP address and port to bind the server to.
    #[arg(short, long, default_value = "0.0.0.0:8080")]
    addr: String,

    /// The connection URI for the MongoDB instance.
    #[arg(long, env = "MONGODB_URI")]
    mongodb_uri: String,

    /// The connection URL for the Redis primary instance.
    #[arg(long, env = "REDIS_PRIMARY_URL")]
    redis_primary_url: String,

    /// The connection URL for the Redis replica instance (optional).
    /// Used for read operations if provided.
    #[arg(long, env = "REDIS_REPLICA_URL")]
    redis_replica_url: Option<String>,

    /// The gRPC endpoint for the User service.
    #[arg(long, env = "USER_SERVICE_ADDR")]
    user_service_addr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    let service = SocialGraphService::new(
        &args.mongodb_uri,
        &args.redis_primary_url,
        args.redis_replica_url.as_deref(),
        &args.user_service_addr,
    )
    .await?;

    let addr = args.addr.parse()?;
    info!("SocialGraphService listening on {}", addr);

    Server::builder()
        .add_service(SocialGraphServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
