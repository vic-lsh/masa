use crate::server::home_timeline::home_timeline_service_server::HomeTimelineServiceServer;
use log::info;
use std::env;
use tonic::transport::Server;
use tracing::Level;

mod server;
// Import the Service struct and the Args struct we defined in server.rs
use server::{Args as ServiceArgs, HomeTimelineService};
use tracing_subscriber::FmtSubscriber;

use deadpool_redis::redis;
use deadpool_redis::{Config, Pool, Runtime};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use tracing for consistent logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // 1. Load Local Configuration (Listen Address & Redis) directly from Env
    let listen_addr =
        env::var("HOME_TIMELINE_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let redis_url =
        env::var("HOME_TIMELINE_REDIS_URL").expect("HOME_TIMELINE_REDIS_URL must be set");

    // 2. Load Downstream Service Configuration (IPs, Ports, Replicas)
    // This uses the logic we added to server.rs
    let service_args = ServiceArgs::from_env()?;

    println!("Creating redis pool...");

    // --- Create Deadpool Redis Pool ---
    let cfg = Config::from_url(redis_url);
    let redis_pool = cfg.create_pool(Some(Runtime::Tokio1))?;
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

    println!("creating service...");

    // 3. Initialize the Service
    // We pass the pool and the service_args (which contains the replica info)
    let service = HomeTimelineService::new(redis_pool, &service_args).await?;

    let addr = listen_addr.parse()?;
    println!("HomeTimelineService listening on {}", addr);

    Server::builder()
        .add_service(HomeTimelineServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
