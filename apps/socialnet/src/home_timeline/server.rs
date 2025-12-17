use deadpool_redis::redis::AsyncCommands;
use deadpool_redis::{Connection, Pool};
use log::{error, warn};
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

use tonic::transport::masa_channel::LoadBalancedChannel;

use std::env;

use socialnet::user_timeline;

#[derive(Clone, Debug)]
pub struct Args {
    // Post Storage Config
    pub post_storage_ip: String,
    pub post_storage_port: u16,
    pub post_storage_replicas: u8,

    // Social Graph Config
    pub social_graph_ip: String,
    pub social_graph_port: u16,
    pub social_graph_replicas: u8,
}

impl Args {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            post_storage_ip: env::var("POST_STORAGE_SERVICE_IP")
                .unwrap_or_else(|_| "socialnet-post-storage-service".to_string()),
            post_storage_port: env::var("POST_STORAGE_SERVICE_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()?,
            post_storage_replicas: env::var("POST_STORAGE_SERVICE_REPLICAS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()?,

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

#[derive(Clone)]
pub struct HomeTimelineService {
    redis_pool: Pool,
    // CHANGED: Use typed Clients with LoadBalancedChannel
    post_storage_client: PostStorageServiceClient<LoadBalancedChannel>,
    social_graph_client: SocialGraphServiceClient<LoadBalancedChannel>,
}

impl HomeTimelineService {
    /// Creates a new instance of the HomeTimelineService.
    pub async fn new(
        redis_pool: Pool,
        args: &Args, // CHANGED: Accept Args struct
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Initialize Post Storage Client
        let post_storage_channel = LoadBalancedChannel::new(
            args.post_storage_ip.clone(),
            args.post_storage_port,
            args.post_storage_replicas,
        )
        .await;
        let post_storage_client = PostStorageServiceClient::new(post_storage_channel);

        // Initialize Social Graph Client
        let social_graph_channel = LoadBalancedChannel::new(
            args.social_graph_ip.clone(),
            args.social_graph_port,
            args.social_graph_replicas,
        )
        .await;
        let social_graph_client = SocialGraphServiceClient::new(social_graph_channel);

        Ok(Self {
            redis_pool,
            post_storage_client,
            social_graph_client,
        })
    }

    /// --- Get a connection from the pool ---
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

        // NEW CHANGE
        let mut client = self.post_storage_client.clone();

        let posts_request = ReadPostsRequest {
            req_id: req.req_id,
            post_ids,
            carrier: req.carrier,
        };
        let posts_response = client.read_posts(posts_request).await?;

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

        // CHANGED: Use existing client from struct, do not connect on every request
        let mut client = self.social_graph_client.clone();

        let followers_request = GetFollowersRequest {
            req_id: req.req_id,
            user_id: req.user_id,
            carrier: req.carrier,
        };
        let followers_response = client.get_followers(followers_request).await?;
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

        // Explicitly tell the compiler the Connection type is inferred `_` and Return type is `()`
        pipe.query_async::<_, ()>(&mut redis_conn)
            .await
            .map_err(|e| {
                error!("Redis pipeline ZADD failed: {}", e);
                Status::internal("Failed to write timeline to cache")
            })?;

        Ok(Response::new(WriteHomeTimelineResponse {}))
    }
}
