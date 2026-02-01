//! Async Rust port of the C++ Write-Home-Timeline Service.

use anyhow::Result;
use structopt::StructOpt;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use deadpool_redis::redis;
use deadpool_redis::Pool as DeadpoolRedisPool;

// NEW IMPORT
use tonic::transport::masa_channel::LoadBalancedChannel;

use crate::worker::run_worker;

pub mod social_graph {
    tonic::include_proto!("social_graph");
}

use crate::social_graph::social_graph_service_client::SocialGraphServiceClient;

pub type RedisPool = DeadpoolRedisPool;
// CHANGED: We don't need bb8 anymore. The Client itself is cheap to clone.
pub type SocialGraphClient = SocialGraphServiceClient<LoadBalancedChannel>;

mod worker;

#[derive(Clone, Debug, StructOpt)]
pub struct Args {
    #[structopt(long, env = "SOCIAL_GRAPH_SERVICE_IP", default_value = "socialnet-social-graph-service")]
    pub social_graph_ip: String,
    #[structopt(long, env = "SOCIAL_GRAPH_SERVICE_PORT", default_value = "8080")]
    pub social_graph_port: u16,
    #[structopt(long, env = "SOCIAL_GRAPH_SERVICE_REPLICAS", default_value = "1")]
    pub social_graph_replicas: u8,

    #[structopt(long, env = "REDIS_URL")]
    pub redis_url: String,

    #[structopt(long, env = "RABBITMQ_URL")]
    pub rabbitmq_url: String,

    #[structopt(long, env = "WORKERS", default_value = "4")]
    pub workers: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let args = Args::from_args();

    let cfg = deadpool_redis::Config::from_url(&args.redis_url);
    let redis_pool = cfg
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .expect("Failed to create Redis pool");
    info!("Redis connection pool created.");

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
        info!("Successfully tested Redis connection pool.");
    }

    // 2. Initialize Social Graph Client (Load Balanced)
    let sg_channel = LoadBalancedChannel::new(
        args.social_graph_ip,
        args.social_graph_port,
        args.social_graph_replicas,
    )
    .await;

    let social_graph_client = SocialGraphServiceClient::new(sg_channel);
    info!("Social Graph Service client created (Load Balanced).");

    // --- Spawn Worker Tasks ---
    let num_workers = args.workers;
    let mut worker_handles = Vec::new();

    for i in 0..num_workers {
        let worker_redis_pool = redis_pool.clone();
        // Just clone the client, it's cheap and thread-safe
        let worker_sg_client = social_graph_client.clone();

        let handle =
            tokio::spawn(async move { run_worker(i, worker_redis_pool, worker_sg_client).await });
        worker_handles.push(handle);
    }
    info!("Spawned {} worker tasks.", num_workers);

    // --- Wait for shutdown signal (Ctrl-C) ---
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Ctrl-C received, shutting down.");
        }
        res = async {
            for handle in worker_handles {
                if let Err(e) = handle.await {
                    return Some(e)
                }
            }
            None
        } => {
            if let Some(e) = res {
                 error!("A worker task panicked or was cancelled: {:?}", e);
            }
        }
    }

    Ok(())
}