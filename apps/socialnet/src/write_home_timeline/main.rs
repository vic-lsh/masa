//! Async Rust port of the C++ Write-Home-Timeline Service.

use anyhow::{Context, Result};
use std::env; 
use tokio::task::JoinHandle;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use deadpool_redis::{Config, Pool as DeadpoolRedisPool, Runtime};
use deadpool_redis::redis;

// NEW IMPORT
use tonic::transport::masa_channel::LoadBalancedChannel;

use crate::worker::run_worker;

pub mod social_graph {
    tonic::include_proto!("social_graph");
}

use crate::social_graph::social_graph_service_client::SocialGraphServiceClient;
use crate::social_graph::GetFollowersRequest;

pub type RedisPool = DeadpoolRedisPool; 
// CHANGED: We don't need bb8 anymore. The Client itself is cheap to clone.
pub type SocialGraphClient = SocialGraphServiceClient<LoadBalancedChannel>;

// --- NEW ARGS STRUCT ---
#[derive(Clone, Debug)]
pub struct Args {
    pub social_graph_ip: String,
    pub social_graph_port: u16,
    pub social_graph_replicas: u8,
}

impl Args {
    pub fn from_env() -> Result<Self, anyhow::Error> {
        Ok(Self {
            social_graph_ip: env::var("SOCIAL_GRAPH_SERVICE_IP")
                .unwrap_or_else(|_| "socialnet-social-graph-service".to_string()),
            social_graph_port: env::var("SOCIAL_GRAPH_SERVICE_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()?,
            social_graph_replicas: env::var("SOCIAL_GRAPH_SERVICE_REPLICAS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()?,
        })
    }
}

mod worker {
    use super::{GetFollowersRequest, RedisPool, SocialGraphClient};
    use super::redis; 
    use anyhow::{anyhow, Context, Result};
    use futures_lite::stream::StreamExt;
    use lapin::{
        options::{BasicAckOptions, BasicConsumeOptions, BasicNackOptions, QueueDeclareOptions},
        types::FieldTable,
        Connection, ConnectionProperties,
    };
    use serde::Deserialize;
    use std::{collections::HashSet, env};
    use tracing::{debug, error, info, warn};

    const QUEUE_NAME: &str = "write-home-timeline";

    #[derive(Debug, Deserialize)]
    struct PostMessage {
        user_id: i64,
        post_id: i64,
        timestamp: i64,
        user_mentions_id: Vec<i64>,
    }
    
    pub async fn run_worker(
        worker_id: usize,
        redis_pool: RedisPool,
        // CHANGED: Pass the client directly
        social_graph_client: SocialGraphClient,
    ) -> Result<()> {
        let rabbit_addr = env::var("RABBITMQ_URL").expect("RABBITMQ_URL must be set");

        info!(worker_id, "Connecting to RabbitMQ at {}", rabbit_addr);
        let conn = Connection::connect(&rabbit_addr, ConnectionProperties::default())
            .await
            .context("Failed to connect to RabbitMQ")?;
        
        let channel = conn
            .create_channel()
            .await
            .context("Failed to create channel")?;

        channel
            .queue_declare(
                QUEUE_NAME,
                QueueDeclareOptions {
                    durable: true,
                    ..Default::default()
                },
                FieldTable::default(),
            )
            .await
            .context("Failed to declare queue")?;

        info!(
            worker_id,
            "Queue '{}' declared. Starting consumption.", QUEUE_NAME
        );

        let mut consumer = channel
            .basic_consume(
                QUEUE_NAME,
                &format!("worker-{}", worker_id),
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;
        
        while let Some(delivery_result) = consumer.next().await {
            let delivery = match delivery_result {
                Ok(d) => d,
                Err(e) => {
                    error!(worker_id, "Error in message delivery: {}", e);
                    continue;
                }
            };

            let redis_clone = redis_pool.clone();
            let social_graph_clone = social_graph_client.clone();
            let channel_clone = channel.clone();

            tokio::spawn(async move {
                let delivery_tag = delivery.delivery_tag;
                match process_message(&delivery.data, redis_clone, social_graph_clone).await {
                    Ok(_) => {
                        if let Err(e) = channel_clone
                            .basic_ack(delivery_tag, BasicAckOptions::default())
                            .await
                        {
                            error!("Failed to ACK message {}: {}", delivery_tag, e);
                        }
                    }
                    Err(e) => {
                        error!("Failed to process message {}: {:?}", delivery_tag, e);
                        if let Err(e) = channel_clone
                            .basic_nack(
                                delivery_tag,
                                BasicNackOptions {
                                    requeue: true,
                                    ..Default::default()
                                },
                            )
                            .await
                        {
                            error!("Failed to NACK message {}: {}", delivery_tag, e);
                        }
                    }
                }
            });
        }
        Ok(())
    }
    
    async fn process_message(
        data: &[u8],
        redis_pool: RedisPool,
        mut social_graph_client: SocialGraphClient,
    ) -> Result<()> {
        let msg: PostMessage =
            serde_json::from_slice(data).context("Failed to parse message JSON")?;

        debug!("Processing message for user_id: {}", msg.user_id);

        // 1. Get followers from the social graph service.
        // CHANGED: Call client directly, no pool.get() needed
        let request = tonic::Request::new(GetFollowersRequest {
            req_id: 0,
            user_id: msg.user_id,
            carrier: Default::default(),
        });

        let followers = match social_graph_client.get_followers(request).await {
            Ok(response) => response.into_inner().user_ids,
            Err(status) => {
                return Err(anyhow!("gRPC call to GetFollowers failed: {}", status));
            }
        };

        // 2. Combine followers and mentioned users.
        let mut user_ids_to_update: HashSet<i64> = followers.into_iter().collect();
        for mentioned_id in msg.user_mentions_id {
            user_ids_to_update.insert(mentioned_id);
        }

        if user_ids_to_update.is_empty() {
            warn!(
                "No followers or mentions for user_id {}, nothing to do.",
                msg.user_id
            );
            return Ok(());
        }

        // 3. Update Redis timelines.
        let mut redis_conn = redis_pool
            .get()
            .await
            .context("Failed to get Redis connection from pool")?;
        
        let mut pipe = redis::pipe();

        for user_id in &user_ids_to_update {
            pipe.zadd(user_id.to_string(), msg.post_id.to_string(), msg.timestamp);
        }

        pipe.query_async::<_, ()>(&mut *redis_conn) 
            .await
            .context("Redis pipeline command failed")?;

        info!(
            "Wrote post {} to timelines of {} users.",
            msg.post_id,
            user_ids_to_update.len()
        );

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // 1. Load Args
    let args = Args::from_env()?;

    let redis_url = env::var("REDIS_URL").expect("REDIS_URL must be set");
    let cfg = deadpool_redis::Config::from_url(redis_url);
    let redis_pool = cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .context("Failed to create Redis pool")?;
    info!("Redis connection pool created.");

    // Test the pool
    {
        let mut conn = redis_pool.get().await.expect("Failed to get Redis connection");
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
        args.social_graph_replicas
    ).await;
    
    let social_graph_client = SocialGraphServiceClient::new(sg_channel);
    info!("Social Graph Service client created (Load Balanced).");

    // --- Spawn Worker Tasks ---
    let num_workers: usize = env::var("WORKERS")
        .unwrap_or_else(|_| "4".to_string())
        .parse()?;
    let mut worker_handles: Vec<JoinHandle<Result<()>>> = Vec::new();
    
    for i in 0..num_workers {
        let worker_redis_pool = redis_pool.clone();
        // Just clone the client, it's cheap and thread-safe
        let worker_sg_client = social_graph_client.clone();

        let handle = tokio::spawn(async move {
            run_worker(i, worker_redis_pool, worker_sg_client).await
        });
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
