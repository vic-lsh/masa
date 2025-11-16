use crate::server::home_timeline::home_timeline_service_server::HomeTimelineServiceServer;
use log::info;
use tonic::transport::Server;
use std::env;
use tracing::Level; 

mod server;
use server::HomeTimelineService;
use tracing_subscriber::FmtSubscriber;

use deadpool_redis::{Config, Pool, Runtime};
use deadpool_redis::redis;


/// Configuration for the home timeline service.
#[derive(Debug, Clone)]
struct Args {
    addr: String,
    redis_url: String, // Simplified to one URL for deadpool
    post_storage_service_addr: String,
    social_graph_service_addr: String,
}

impl Args {
    /// Load configuration from environment variables.
    fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            addr: env::var("HOME_TIMElINE_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            
            // Use the REDIS_URL from your docker-compose
            redis_url: env::var("HOME_TIMELINE_REDIS_URL")
                .expect("HOME_TIMELINE_REDIS_URL must be set"),
            
            post_storage_service_addr: env::var("POST_STORAGE_SERVICE_ADDR")
                .expect("POST_STORAGE_SERVICE_ADDR must be set"),
            
            social_graph_service_addr: env::var("SOCIAL_GRAPH_SERVICE_ADDR")
                .expect("SOCIAL_GRAPH_SERVICE_ADDR must be set"),
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use tracing for consistent logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
    let args = Args::from_env()?;

    println!("Creating redis pool...");

    // --- Create Deadpool Redis Pool ---
    let cfg = Config::from_url(args.redis_url.clone()); 
    let redis_pool = cfg.create_pool(Some(Runtime::Tokio1))?;
    println!("Successfully created Redis connection pool.");

    // Test the pool
    {
        let mut conn = redis_pool.get().await.expect("Failed to get Redis connection");

        let _: String = deadpool_redis::redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .expect("Redis PING failed");
        println!("Successfully tested Redis connection pool.");
    }

    println!("creating service...");
    let service = HomeTimelineService::new(
        redis_pool, // Pass the pool
        &args.post_storage_service_addr,
        &args.social_graph_service_addr,
    )
    .await?;


    let addr = args.addr.parse()?;
    println!("HomeTimelineService listening on {}", addr);

    Server::builder()
        .add_service(HomeTimelineServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
