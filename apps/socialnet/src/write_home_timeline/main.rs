//! Async Rust port of the C++ Write-Home-Timeline Service.

use anyhow::{Context, Result};
use std::env; // <-- Make sure this is imported
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use crate::worker::run_worker;

pub mod social_graph {
    tonic::include_proto!("social_graph");
}

use crate::social_graph::social_graph_service_client::SocialGraphServiceClient;
use crate::social_graph::GetFollowersRequest;
use tonic::transport::Channel;

pub type RedisPool = bb8::Pool<bb8_redis::RedisConnectionManager>;
pub type SocialGraphPool = bb8::Pool<social_graph_client::SocialGraphConnectionManager>;

// The entire 'app_config' module can be removed as we are now using environment variables.

mod social_graph_client {
    // ... (this module does not need to change)
    use super::{Channel, SocialGraphServiceClient};
    use anyhow::{anyhow, Result};
    use tonic::transport::Endpoint;

    #[derive(Debug)]
    pub struct MyError(pub anyhow::Error);

    impl std::fmt::Display for MyError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.0.fmt(f)
        }
    }

    impl std::error::Error for MyError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.0.source()
        }
    }
    
    #[derive(Clone, Debug)]
    pub struct SocialGraphConnectionManager {
        pub addr: String,
    }

    impl bb8::ManageConnection for SocialGraphConnectionManager {
        type Connection = SocialGraphServiceClient<Channel>;
        type Error = MyError;

        async fn connect(&self) -> Result<Self::Connection, Self::Error> {
            let channel = Endpoint::from_shared(self.addr.clone())
                .map_err(|e| MyError(anyhow!(e)))?
                .connect()
                .await
                .map_err(|e| MyError(anyhow!("Failed to connect to Social Graph Service: {}", e)))?;
            Ok(SocialGraphServiceClient::new(channel))
        }

        async fn is_valid(&self, _conn: &mut Self::Connection) -> Result<(), Self::Error> {
            Ok(())
        }

        fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
            false
        }
    }
}

mod worker {
    // ... (structs PostMessage, QUEUE_NAME, and process_message function are unchanged) ...
    use super::{GetFollowersRequest, RedisPool, SocialGraphPool};
    use anyhow::{anyhow, Context, Result};
    use futures_lite::stream::StreamExt;
    use lapin::{
        options::{BasicAckOptions, BasicConsumeOptions, BasicNackOptions, QueueDeclareOptions},
        types::FieldTable,
        Connection, ConnectionProperties,
    };
    use serde::Deserialize;
    use std::{collections::HashSet, env, sync::Arc}; // <-- Add env here
    use tracing::{debug, error, info, warn};

    const QUEUE_NAME: &str = "write-home-timeline";

    #[derive(Debug, Deserialize)]
    struct PostMessage {
        user_id: i64,
        post_id: i64,
        timestamp: i64,
        user_mentions_id: Vec<i64>,
    }
    
    // CHANGED: The 'config' argument has been removed from the function signature.
    pub async fn run_worker(
        worker_id: usize,
        redis_pool: RedisPool,
        social_graph_pool: SocialGraphPool,
    ) -> Result<()> {
        // CHANGED: The RabbitMQ address is now read directly from an environment variable.
        let rabbit_addr = env::var("RABBITMQ_URL").expect("RABBITMQ_URL must be set");

        info!(worker_id, "Connecting to RabbitMQ at {}", rabbit_addr);
        let conn = Connection::connect(&rabbit_addr, ConnectionProperties::default())
            .await
            .context("Failed to connect to RabbitMQ")?;
        
        // ... (rest of the run_worker function is the same) ...
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
            let social_graph_clone = social_graph_pool.clone();
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
        social_graph_pool: SocialGraphPool,
    ) -> Result<()> {
        let msg: PostMessage =
            serde_json::from_slice(data).context("Failed to parse message JSON")?;

        debug!("Processing message for user_id: {}", msg.user_id);

        // 1. Get followers from the social graph service.
        let mut social_graph_conn = social_graph_pool
            .get()
            .await
            .context("Failed to get social graph client from pool")?;

        let request = tonic::Request::new(GetFollowersRequest {
            req_id: 0,
            user_id: msg.user_id,
            carrier: Default::default(),
        });

        let followers = match social_graph_conn.get_followers(request).await {
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

        pipe.query_async::<()>(&mut *redis_conn)
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

    // CHANGED: Removed all file-based config loading.

    // --- Create Connection Pools using environment variables ---
    let redis_url = env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis_manager = bb8_redis::RedisConnectionManager::new(redis_url.as_str())?;
    let redis_pool = bb8::Pool::builder()
        .max_size(10) // Using a sensible default
        .build(redis_manager)
        .await
        .context("Failed to create Redis pool")?;
    info!("Redis connection pool created.");

    let sg_addr = env::var("SOCIAL_GRAPH_SERVICE_ADDR").expect("SOCIAL_GRAPH_SERVICE_ADDR must be set");
    let sg_manager = social_graph_client::SocialGraphConnectionManager { addr: sg_addr };
    let social_graph_pool = bb8::Pool::builder()
        .max_size(10) // Using a sensible default
        .build(sg_manager)
        .await
        .context("Failed to create Social Graph Service pool")?;
    info!("Social Graph Service connection pool created.");

    // --- Spawn Worker Tasks ---
    // CHANGED: Number of workers is now read from an environment variable.
    let num_workers: usize = env::var("WORKERS")
        .unwrap_or_else(|_| "4".to_string())
        .parse()?;
    let mut worker_handles: Vec<JoinHandle<Result<()>>> = Vec::new();
    
    for i in 0..num_workers {
        let worker_redis_pool = redis_pool.clone();
        let worker_sg_pool = social_graph_pool.clone();

        // CHANGED: The 'worker_config' argument is no longer passed.
        let handle = tokio::spawn(async move {
            run_worker(i, worker_redis_pool, worker_sg_pool).await
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
