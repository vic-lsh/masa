use crate::server::social_graph::social_graph_service_server::SocialGraphServiceServer;
use log::info;
use tonic::transport::Server;
use std::env;
use tracing::Level; // Use tracing for logging
use tracing_subscriber::FmtSubscriber;

mod server;
use server::SocialGraphService;

use deadpool_redis::{Config, Pool, Runtime};
use deadpool_redis::redis;


/// The command-line arguments for the social graph service.
#[derive(Debug, Clone)]
struct Args {
    addr: String,
    mongodb_uri: String,
    redis_url: String, // Simplified to a single URL for deadpool
    user_service_addr: String,
}

impl Args {
    /// Load configuration from environment variables.
    fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            addr: env::var("SOCIAL_GRAPH_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            
            mongodb_uri: env::var("MONGO_URL")
                .expect("MONGO_URL must be set"),
            
            redis_url: env::var("REDIS_URL")
                .expect("REDIS_URL must be set"),
            
            user_service_addr: env::var("USER_SERVICE_ADDR")
                .expect("USER_SERVICE_ADDR must be set"),
        })
    }
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use tracing for consistent logging
    // tracing_subscriber::fmt::init();
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
    let args = Args::from_env().expect("env not passed correctly");

    // --- Create Deadpool Redis Pool ---
    let cfg = Config::from_url(args.redis_url.clone()); // removed mut
    let redis_pool = cfg.create_pool(Some(Runtime::Tokio1)).expect("pool failed");
    println!("Successfully created Redis connection pool.");

    // Test the pool
    {
        let mut conn = redis_pool.get().await.expect("Failed to get Redis connection");
        // --- THIS IS THE FIX ---
        // Use the re-exported cmd and expect a String reply
        let _: String = deadpool_redis::redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .expect("Redis PING failed");
        println!("Successfully tested Redis connection pool.");
    }
    
    // Pass the pool to the service
    let service = SocialGraphService::new(
        &args.mongodb_uri,
        redis_pool, // Pass the whole pool
        &args.user_service_addr,
    )
    .await.expect("social graph failed");

    let addr = args.addr.parse().expect("incorrect parsing");
    println!("SocialGraphService listening on {}", addr);

    Server::builder()
        .add_service(SocialGraphServiceServer::new(service))
        .serve(addr)
        .await
        .expect("build failed");

    Ok(())
}
