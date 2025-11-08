use log::{error, warn};
use deadpool_redis::redis::{self, AsyncCommands, Client as RedisClient};
use deadpool_redis::{Pool, Connection};
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

// --- REMOVED RedisManager ENUM ---

/// The implementation of the HomeTimeline gRPC service.
pub struct HomeTimelineService {
    redis_pool: Pool,
    post_storage_addr: String,
    social_graph_addr: String,
}

impl HomeTimelineService {
    /// Creates a new instance of the HomeTimelineService.
    pub async fn new(
        redis_pool: Pool,
        post_storage_addr: &str,
        social_graph_addr: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            redis_pool,
            post_storage_addr: post_storage_addr.to_string(),
            social_graph_addr: social_graph_addr.to_string(),
        })
    }

    /// --- NEW HELPER: Get a connection from the pool ---
    async fn get_conn(&self) -> Result<Connection, Status> {
        self.redis_pool.get().await.map_err(|e| {
            error!("Failed to get Redis connection from pool: {}", e);
            Status::internal("Cache service unavailable")
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

        let mut redis_conn = self.get_conn().await?;
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
            PostStorageServiceClient::connect(self.post_storage_addr.clone())
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
            SocialGraphServiceClient::connect(self.social_graph_addr.clone())
                .await
                .map_err(|e| {
                    error!("Failed to connect to SocialGraphService: {}", e);
                    Status::internal(format!("Failed to connect to SocialGraphService: {}", e))
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

        let mut redis_conn = self.get_conn().await?;
        let mut pipe = deadpool_redis::redis::pipe();
        pipe.atomic(); // Make it a transaction.

        for user_id in user_ids_to_update {
            pipe.zadd(user_id.to_string(), req.post_id.to_string(), req.timestamp);
        }

        // --- THIS IS THE FIX ---
        // Explicitly tell the compiler the Connection type is inferred `_`
        // and the Return type is `()`.
        pipe.query_async::<_, ()>(&mut redis_conn).await.map_err(|e| {
            error!("Redis pipeline ZADD failed: {}", e);
            Status::internal("Failed to write timeline to cache")
        })?;

        Ok(Response::new(WriteHomeTimelineResponse {}))
    }
}
