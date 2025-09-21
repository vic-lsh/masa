use log::{error, warn};
use redis::AsyncCommands;
use std::collections::HashSet;
use tonic::{Request, Response, Status};

pub mod home_timeline {
    tonic::include_proto!("home_timeline");
}
pub mod post_storage {
    tonic::include_proto!("post_storage");
}
pub mod social_graph {
    tonic::include_proto!("social_graph");
}

use home_timeline::{
    home_timeline_service_server::HomeTimelineService as GrpcService, ReadHomeTimelineRequest,
    ReadHomeTimelineResponse, WriteHomeTimelineRequest, WriteHomeTimelineResponse,
};
use post_storage::post_storage_service_client::PostStorageServiceClient;
use post_storage::ReadPostsRequest;
use social_graph::social_graph_service_client::SocialGraphServiceClient;
use social_graph::GetFollowersRequest;

use socialnet::user_timeline;

/// An enum to manage different Redis client configurations.
enum RedisManager {
    /// A single client for both reads and writes.
    Single(redis::Client),
    /// Separate clients for primary (writes) and replica (reads).
    Replica {
        primary: redis::Client,
        replica: redis::Client,
    },
}

impl RedisManager {
    /// Gets a connection for write operations.
    async fn get_write_conn(&self) -> Result<redis::aio::MultiplexedConnection, Status> {
        let client = match self {
            RedisManager::Single(c) => c,
            RedisManager::Replica { primary, .. } => primary,
        };
        client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| {
                error!("Failed to connect to Redis for write: {}", e);
                Status::internal("Failed to connect to Redis")
            })
    }

    /// Gets a connection for read operations. Prefers replica if available.
    async fn get_read_conn(&self) -> Result<redis::aio::MultiplexedConnection, Status> {
        let client = match self {
            RedisManager::Single(c) => c,
            RedisManager::Replica { replica, .. } => replica,
        };
        client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| {
                error!("Failed to connect to Redis for read: {}", e);
                Status::internal("Failed to connect to Redis")
            })
    }
}

/// The implementation of the HomeTimeline gRPC service.
pub struct HomeTimelineService {
    redis_manager: RedisManager,
    post_storage_addr: String,
    social_graph_addr: String,
}

impl HomeTimelineService {
    /// Creates a new instance of the HomeTimelineService.
    pub async fn new(
        primary_redis_url: &str,
        replica_redis_url: Option<&str>,
        post_storage_addr: &str,
        social_graph_addr: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let redis_manager = match replica_redis_url {
            Some(replica_url) => RedisManager::Replica {
                primary: redis::Client::open(primary_redis_url)?,
                replica: redis::Client::open(replica_url)?,
            },
            None => RedisManager::Single(redis::Client::open(primary_redis_url)?),
        };

        Ok(Self {
            redis_manager,
            post_storage_addr: post_storage_addr.to_string(),
            social_graph_addr: social_graph_addr.to_string(),
        })
    }
}

#[tonic::async_trait]
impl GrpcService for HomeTimelineService {
    /// Handles the ReadHomeTimeline RPC call.
    async fn read_home_timeline(
        &self,
        request: Request<ReadHomeTimelineRequest>,
    ) -> Result<Response<ReadHomeTimelineResponse>, Status> {
        let req = request.into_inner();

        if req.stop <= req.start || req.start < 0 {
            warn!(
                "Invalid start/stop indices: start={}, stop={}",
                req.start, req.stop
            );
            return Ok(Response::new(ReadHomeTimelineResponse::default()));
        }

        let mut redis_conn = self.redis_manager.get_read_conn().await?;
        let user_key = req.user_id.to_string();

        let post_ids_str: Vec<String> = redis_conn
            .zrevrange(&user_key, req.start as isize, req.stop as isize - 1)
            .await
            .map_err(|e| {
                error!("Redis ZREVRANGE failed for user {}: {}", req.user_id, e);
                Status::internal("Failed to read timeline from cache")
            })?;

        let post_ids: Vec<i64> = post_ids_str
            .into_iter()
            .filter_map(|s| s.parse::<i64>().ok())
            .collect();

        if post_ids.is_empty() {
            return Ok(Response::new(ReadHomeTimelineResponse::default()));
        }

        let mut post_storage_client =
            PostStorageServiceClient::connect(format!("http://{}", self.post_storage_addr))
                .await
                .map_err(|e| {
                    error!("Failed to connect to PostStorageService: {}", e);
                    Status::internal("Failed to connect to PostStorageService")
                })?;

        let posts_request = ReadPostsRequest {
            req_id: req.req_id,
            post_ids,
            carrier: req.carrier,
        };
        let posts_response = post_storage_client.read_posts(posts_request).await?;

        Ok(Response::new(ReadHomeTimelineResponse {
            posts: posts_response.into_inner().posts,
        }))
    }

    /// Handles the WriteHomeTimeline RPC call.
    async fn write_home_timeline(
        &self,
        request: Request<WriteHomeTimelineRequest>,
    ) -> Result<Response<WriteHomeTimelineResponse>, Status> {
        let req = request.into_inner();

        let mut social_graph_client =
            SocialGraphServiceClient::connect(format!("http://{}", self.social_graph_addr))
                .await
                .map_err(|e| {
                    error!("Failed to connect to SocialGraphService: {}", e);
                    Status::internal("Failed to connect to SocialGraphService")
                })?;

        let followers_request = GetFollowersRequest {
            req_id: req.req_id,
            user_id: req.user_id,
            carrier: req.carrier,
        };
        let followers_response = social_graph_client.get_followers(followers_request).await?;
        let follower_ids = followers_response.into_inner().user_ids;

        let mut user_ids_to_update: HashSet<i64> = follower_ids.into_iter().collect();
        for mention_id in req.user_mentions_id {
            user_ids_to_update.insert(mention_id);
        }

        if user_ids_to_update.is_empty() {
            return Ok(Response::new(WriteHomeTimelineResponse {}));
        }

        let mut redis_conn = self.redis_manager.get_write_conn().await?;
        let mut pipe = redis::pipe();
        pipe.atomic(); // Make it a transaction.

        for user_id in user_ids_to_update {
            pipe.zadd(user_id.to_string(), req.post_id.to_string(), req.timestamp);
        }

        pipe.query_async::<()>(&mut redis_conn).await.map_err(|e| {
            error!("Redis pipeline ZADD failed: {}", e);
            Status::internal("Failed to write timeline to cache")
        })?;

        Ok(Response::new(WriteHomeTimelineResponse {}))
    }
}
