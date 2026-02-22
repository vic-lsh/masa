use crate::server::social_graph::social_graph_service_server::SocialGraphServiceServer;
use std::env;
use tonic::transport::Server;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

mod server;
// Import the Service and the Args struct defined in server.rs
use server::{Args as ServiceArgs, SocialGraphService};

use deadpool_redis::{Config, Runtime};

/// The command-line arguments for the social graph service (Local Config).
#[derive(Debug, Clone)]
struct LocalArgs {
    addr: String,
    mongodb_uri: String,
    redis_url: String,
    // Removed user_service_addr: It is now handled by ServiceArgs in server.rs
}

impl LocalArgs {
    /// Load local configuration from environment variables.
    fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            addr: env::var("SOCIAL_GRAPH_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),

            mongodb_uri: env::var("MONGO_URL").expect("MONGO_URL must be set"),

            redis_url: env::var("REDIS_URL").expect("REDIS_URL must be set"),
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use tracing for consistent logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // 1. Load Local Config (DBs, Listen Addr)
    let local_args = LocalArgs::from_env()?;

    // 2. Load Service Config (Replica IPs/Ports for User Service)
    let service_args = ServiceArgs::from_env()?;

    // --- Create Deadpool Redis Pool ---
    let cfg = Config::from_url(local_args.redis_url.clone());
    let redis_pool = cfg.create_pool(Some(Runtime::Tokio1)).expect("pool failed");
    println!("Successfully created Redis connection pool.");

    // Test the pool
    {
        let mut conn = redis_pool
            .get()
            .await
            .expect("Failed to get Redis connection");
        let _: String = deadpool_redis::redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .expect("Redis PING failed");
        println!("Successfully tested Redis connection pool.");
    }

    // 3. Initialize Service
    // Pass mongodb_uri, redis_pool, and the service_args (for user_service connection)
    let service =
        SocialGraphService::new(&local_args.mongodb_uri, redis_pool, &service_args).await?;

    let addr = local_args.addr.parse().expect("incorrect parsing");
    println!("SocialGraphService listening on {}", addr);

    Server::builder()
        .add_service(SocialGraphServiceServer::new(service))
        .serve_with_masa(addr)
        .await?;

    Ok(())
}
