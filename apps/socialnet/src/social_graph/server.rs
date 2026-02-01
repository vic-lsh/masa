use chrono::Utc;
use log::{error, info, warn};
use mongodb::{
    bson::{doc, document::Document},
    Client as MongoClient, Collection,
};
use std::env;

use deadpool_redis::redis::{self, AsyncCommands};
use deadpool_redis::{Connection, Pool};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tonic::{Request, Response, Status};
use structopt::StructOpt;

// NEW IMPORT
use tonic::transport::masa_channel::LoadBalancedChannel;

// gRPC generated modules
pub mod social_graph {
    tonic::include_proto!("social_graph");
}
pub mod user {
    tonic::include_proto!("user");
}

use social_graph::{
    social_graph_service_server::SocialGraphService as GrpcService, FollowRequest, FollowResponse,
    FollowWithUsernameRequest, FollowWithUsernameResponse, GetFolloweesRequest,
    GetFolloweesResponse, GetFollowersRequest, GetFollowersResponse, InsertUserRequest,
    InsertUserResponse, UnfollowRequest, UnfollowResponse, UnfollowWithUsernameRequest,
    UnfollowWithUsernameResponse,
};
use user::user_service_client::UserServiceClient;
use user::GetUserIdRequest;

// --- NEW ARGS STRUCT ---
#[derive(Clone, Debug, StructOpt)]
pub struct Args {
    // User Service Config
    #[structopt(long, env = "USER_SERVICE_IP", default_value = "socialnet-user-service")]
    pub user_service_ip: String,
    #[structopt(long, env = "USER_SERVICE_PORT", default_value = "8080")]
    pub user_service_port: u16,
    #[structopt(long, env = "USER_SERVICE_REPLICAS", default_value = "1")]
    pub user_service_replicas: u8,
}

/// Represents an edge (follower/followee relationship) in the MongoDB document.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Edge {
    user_id: i64,
    timestamp: i64,
}

/// The implementation of the SocialGraph gRPC service.
pub struct SocialGraphService {
    redis_pool: Pool,
    mongo_collection: Collection<Document>,
    // CHANGED: Use typed Client with LoadBalancedChannel
    user_service_client: UserServiceClient<LoadBalancedChannel>,
}

impl SocialGraphService {
    /// Creates a new instance of the SocialGraphService.
    pub async fn new(
        mongodb_uri: &str,
        redis_pool: Pool,
        args: &Args, // CHANGED: Accept Args struct
    ) -> Result<Self, Box<dyn std::error::Error>> {
        println!("initializing mongo");
        let mongo_client = MongoClient::with_uri_str(mongodb_uri)
            .await
            .expect("mongo failed");
        let db = mongo_client.database("social-graph");
        let mongo_collection = db.collection("social-graph");

        // Initialize User Service Client
        let user_service_channel = LoadBalancedChannel::new(
            args.user_service_ip.clone(),
            args.user_service_port,
            args.user_service_replicas,
        )
        .await;
        let user_service_client = UserServiceClient::new(user_service_channel);

        Ok(Self {
            redis_pool,
            mongo_collection,
            user_service_client,
        })
    }

    /// --- NEW HELPER: Get a connection from the pool ---
    async fn get_conn(&self) -> Result<Connection, Status> {
        // Removed println to reduce log spam on high load
        self.redis_pool.get().await.map_err(|e| {
            error!("Failed to get Redis connection from pool: {}", e);
            Status::internal("Cache service unavailable")
        })
    }

    /// Resolves a username to a user ID by calling the UserService.
    async fn get_user_id(
        &self,
        username: String,
        req_id: i64,
        carrier: HashMap<String, String>,
    ) -> Result<i64, Status> {
        // CHANGED: Use the pre-initialized client
        let mut user_client = self.user_service_client.clone();

        let request = GetUserIdRequest {
            req_id,
            username,
            carrier,
        };

        let response = user_client.get_user_id(request).await?;
        Ok(response.into_inner().user_id)
    }

    /// Internal logic to add a follow relationship, updating both MongoDB and Redis.
    async fn _follow(&self, user_id: i64, followee_id: i64) -> Result<(), Status> {
        let timestamp = Utc::now().timestamp_millis();

        let redis_update = async {
            let mut conn = self.get_conn().await?;
            let user_followees_key = format!("{}:followees", user_id);
            let followee_followers_key = format!("{}:followers", followee_id);

            let _: () = redis::pipe()
                .cmd("ZADD")
                .arg(&user_followees_key)
                .arg("NX")
                .arg(timestamp)
                .arg(followee_id)
                .cmd("ZADD")
                .arg(&followee_followers_key)
                .arg("NX")
                .arg(timestamp)
                .arg(user_id)
                .query_async::<_, ()>(&mut conn)
                .await
                .map_err(|e| {
                    error!("Redis ZADD failed for follow: {}", e);
                    Status::internal("Redis follow update failed")
                })?;
            Ok(())
        };

        let mongo_update_user = async {
            let filter = doc! {
                "user_id": user_id,
                "followees.user_id": { "$ne": followee_id }
            };
            let update = doc! { "$push": { "followees": { "user_id": followee_id, "timestamp": timestamp } } };
            self.mongo_collection
                .find_one_and_update(filter, update, None)
                .await
                .map_err(|e| {
                    error!("MongoDB follow update failed for user {}: {}", user_id, e);
                    Status::internal("MongoDB update failed")
                })
        };

        let mongo_update_followee = async {
            let filter = doc! {
                "user_id": followee_id,
                "followers.user_id": { "$ne": user_id }
            };
            let update =
                doc! { "$push": { "followers": { "user_id": user_id, "timestamp": timestamp } } };
            self.mongo_collection
                .find_one_and_update(filter, update, None)
                .await
                .map_err(|e| {
                    error!(
                        "MongoDB follow update failed for followee {}: {}",
                        followee_id, e
                    );
                    Status::internal("MongoDB update failed")
                })
        };

        tokio::try_join!(redis_update, mongo_update_user, mongo_update_followee)?;
        Ok(())
    }

    /// Internal logic to remove a follow relationship.
    async fn _unfollow(&self, user_id: i64, followee_id: i64) -> Result<(), Status> {
        let redis_update = async {
            let mut conn = self.get_conn().await?;
            let user_followees_key = format!("{}:followees", user_id);
            let followee_followers_key = format!("{}:followers", followee_id);

            let _: () = redis::pipe()
                .zrem(user_followees_key, followee_id)
                .zrem(followee_followers_key, user_id)
                .query_async::<_, ()>(&mut conn)
                .await
                .map_err(|e| {
                    error!("Redis ZREM failed for unfollow: {}", e);
                    Status::internal("Redis unfollow update failed")
                })?;
            Ok(())
        };

        let mongo_update_user = async {
            let filter = doc! { "user_id": user_id };
            let update = doc! { "$pull": { "followees": { "user_id": followee_id } } };
            self.mongo_collection
                .find_one_and_update(filter, update, None)
                .await
                .map_err(|e| {
                    error!("MongoDB unfollow update failed for user {}: {}", user_id, e);
                    Status::internal("MongoDB update failed")
                })
        };

        let mongo_update_followee = async {
            let filter = doc! { "user_id": followee_id };
            let update = doc! { "$pull": { "followers": { "user_id": user_id } } };
            self.mongo_collection
                .find_one_and_update(filter, update, None)
                .await
                .map_err(|e| {
                    error!(
                        "MongoDB unfollow update failed for followee {}: {}",
                        followee_id, e
                    );
                    Status::internal("MongoDB update failed")
                })
        };

        tokio::try_join!(redis_update, mongo_update_user, mongo_update_followee)?;
        Ok(())
    }
}

#[tonic::async_trait]
impl GrpcService for SocialGraphService {
    async fn get_followers(
        &self,
        request: Request<GetFollowersRequest>,
    ) -> Result<Response<GetFollowersResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id;
        let key = format!("{}:followers", user_id);

        let mut conn = self.get_conn().await?;

        let redis_followers: Vec<i64> = conn.zrange(&key, 0, -1).await.map_err(|e| {
            error!("Redis ZRANGE failed for followers of {}: {}", user_id, e);
            Status::internal("Failed to read from cache")
        })?;

        if !redis_followers.is_empty() {
            info!("Cache hit for followers of user {}", user_id);
            return Ok(Response::new(GetFollowersResponse {
                user_ids: redis_followers,
            }));
        }

        info!(
            "Cache miss for followers of user {}. Fetching from MongoDB.",
            user_id
        );
        let filter = doc! { "user_id": user_id };
        let doc_opt = self
            .mongo_collection
            .find_one(filter, None)
            .await
            .map_err(|e| {
                error!("MongoDB find_one failed for user {}: {}", user_id, e);
                Status::internal("Database read error")
            })?;

        let (followers, redis_zset) = if let Some(doc) = doc_opt {
            let followers_bson = doc.get_array("followers").unwrap_or(&Vec::new()).to_owned();
            let edges: Vec<Edge> =
                mongodb::bson::from_bson(mongodb::bson::Bson::Array(followers_bson))
                    .unwrap_or_default();

            let user_ids: Vec<i64> = edges.iter().map(|e| e.user_id).collect();
            let zset_items: Vec<(i64, i64)> =
                edges.iter().map(|e| (e.timestamp, e.user_id)).collect();

            (user_ids, zset_items)
        } else {
            warn!("User {} not found in MongoDB", user_id);
            (Vec::new(), Vec::new())
        };

        if !redis_zset.is_empty() {
            let pool = self.redis_pool.clone();
            tokio::spawn(async move {
                if let Ok(mut conn) = pool.get().await {
                    match conn
                        .zadd_multiple::<&str, i64, i64, ()>(&key, &redis_zset)
                        .await
                    {
                        Ok(_) => info!("Updated Redis cache for followers of {}", user_id),
                        Err(e) => warn!("Failed to update Redis cache for {}: {}", user_id, e),
                    }
                } else {
                    warn!("Failed to get Redis conn for cache update for {}", user_id);
                }
            });
        }

        Ok(Response::new(GetFollowersResponse {
            user_ids: followers,
        }))
    }

    async fn get_followees(
        &self,
        request: Request<GetFolloweesRequest>,
    ) -> Result<Response<GetFolloweesResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id;
        let key = format!("{}:followees", user_id);

        let mut conn = self.get_conn().await?;

        let redis_followees: Vec<i64> = conn.zrange(&key, 0, -1).await.map_err(|e| {
            error!("Redis ZRANGE failed for followees of {}: {}", user_id, e);
            Status::internal("Failed to read from cache")
        })?;

        if !redis_followees.is_empty() {
            info!("Cache hit for followees of user {}", user_id);
            return Ok(Response::new(GetFolloweesResponse {
                user_ids: redis_followees,
            }));
        }

        info!(
            "Cache miss for followees of user {}. Fetching from MongoDB.",
            user_id
        );
        let filter = doc! { "user_id": user_id };
        let doc_opt = self
            .mongo_collection
            .find_one(filter, None)
            .await
            .map_err(|e| {
                error!("MongoDB find_one failed for user {}: {}", user_id, e);
                Status::internal("Database read error")
            })?;

        let (followees, redis_zset) = if let Some(doc) = doc_opt {
            let followees_bson = doc.get_array("followees").unwrap_or(&Vec::new()).to_owned();
            let edges: Vec<Edge> =
                mongodb::bson::from_bson(mongodb::bson::Bson::Array(followees_bson))
                    .unwrap_or_default();

            let followees: Vec<i64> = edges.iter().map(|e| e.user_id).collect();
            let zset_items: Vec<(i64, i64)> =
                edges.iter().map(|e| (e.timestamp, e.user_id)).collect();

            (followees, zset_items)
        } else {
            warn!("User {} not found in MongoDB", user_id);
            (Vec::new(), Vec::new())
        };

        if !redis_zset.is_empty() {
            let pool = self.redis_pool.clone();
            tokio::spawn(async move {
                if let Ok(mut conn) = pool.get().await {
                    match conn
                        .zadd_multiple::<&str, i64, i64, ()>(&key, &redis_zset)
                        .await
                    {
                        Ok(_) => info!("Updated Redis cache for followees of {}", user_id),
                        Err(e) => warn!("Failed to update Redis cache for {}: {}", user_id, e),
                    }
                } else {
                    warn!("Failed to get Redis conn for cache update for {}", user_id);
                }
            });
        }

        Ok(Response::new(GetFolloweesResponse {
            user_ids: followees,
        }))
    }

    async fn follow(
        &self,
        request: Request<FollowRequest>,
    ) -> Result<Response<FollowResponse>, Status> {
        let req = request.into_inner();
        self._follow(req.user_id, req.followee_id).await?;
        Ok(Response::new(FollowResponse {}))
    }

    async fn unfollow(
        &self,
        request: Request<UnfollowRequest>,
    ) -> Result<Response<UnfollowResponse>, Status> {
        let req = request.into_inner();
        self._unfollow(req.user_id, req.followee_id).await?;
        Ok(Response::new(UnfollowResponse {}))
    }

    async fn follow_with_username(
        &self,
        request: Request<FollowWithUsernameRequest>,
    ) -> Result<Response<FollowWithUsernameResponse>, Status> {
        let req = request.into_inner();

        let user_id_fut = self.get_user_id(req.user_username, req.req_id, req.carrier.clone());
        let followee_id_fut = self.get_user_id(req.followee_username, req.req_id, req.carrier);

        let (user_id_res, followee_id_res) = tokio::join!(user_id_fut, followee_id_fut);

        let user_id = user_id_res?;
        let followee_id = followee_id_res?;

        if user_id >= 0 && followee_id >= 0 {
            self._follow(user_id, followee_id).await?;
        } else {
            return Err(Status::not_found("One or both users not found"));
        }

        Ok(Response::new(FollowWithUsernameResponse {}))
    }

    async fn unfollow_with_username(
        &self,
        request: Request<UnfollowWithUsernameRequest>,
    ) -> Result<Response<UnfollowWithUsernameResponse>, Status> {
        let req = request.into_inner();

        let user_id_fut = self.get_user_id(req.user_username, req.req_id, req.carrier.clone());
        let followee_id_fut = self.get_user_id(req.followee_username, req.req_id, req.carrier);

        let (user_id_res, followee_id_res) = tokio::join!(user_id_fut, followee_id_fut);

        let user_id = user_id_res?;
        let followee_id = followee_id_res?;

        if user_id >= 0 && followee_id >= 0 {
            self._unfollow(user_id, followee_id).await?;
        } else {
            return Err(Status::not_found("One or both users not found"));
        }

        Ok(Response::new(UnfollowWithUsernameResponse {}))
    }

    async fn insert_user(
        &self,
        request: Request<InsertUserRequest>,
    ) -> Result<Response<InsertUserResponse>, Status> {
        let req = request.into_inner();
        let user_id = req.user_id;

        let new_doc = doc! {
            "user_id": user_id,
            "followers": [],
            "followees": []
        };

        self.mongo_collection
            .insert_one(new_doc, None)
            .await
            .map_err(|e| {
                error!("MongoDB insert_one failed for user {}: {}", user_id, e);
                Status::internal("Database insert error")
            })?;

        info!("Successfully inserted user {}", user_id);
        Ok(Response::new(InsertUserResponse {}))
    }
}