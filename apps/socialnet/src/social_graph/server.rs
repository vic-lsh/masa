use chrono::Utc;
use log::{error, info, warn};
use mongodb::{
    bson::{doc, document::Document},
    Client as MongoClient, Collection,
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tonic::{Request, Response, Status};

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

/// Represents an edge (follower/followee relationship) in the MongoDB document.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Edge {
    user_id: i64,
    timestamp: i64,
}

/// The implementation of the SocialGraph gRPC service.
pub struct SocialGraphService {
    redis_manager: RedisManager,
    mongo_collection: Collection<Document>,
    user_service_addr: String,
}

impl SocialGraphService {
    /// Creates a new instance of the SocialGraphService.
    pub async fn new(
        mongodb_uri: &str,
        primary_redis_url: &str,
        replica_redis_url: Option<&str>,
        user_service_addr: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mongo_client = MongoClient::with_uri_str(mongodb_uri).await?;
        let db = mongo_client.database("social-graph");
        let mongo_collection = db.collection("social-graph");

        let redis_manager = match replica_redis_url {
            Some(replica_url) => RedisManager::Replica {
                primary: redis::Client::open(primary_redis_url)?,
                replica: redis::Client::open(replica_url)?,
            },
            None => RedisManager::Single(redis::Client::open(primary_redis_url)?),
        };

        Ok(Self {
            redis_manager,
            mongo_collection,
            user_service_addr: user_service_addr.to_string(),
        })
    }

    /// Resolves a username to a user ID by calling the UserService.
    async fn get_user_id(
        &self,
        username: String,
        req_id: i64,
        carrier: HashMap<String, String>,
    ) -> Result<i64, Status> {
        let mut user_client =
            UserServiceClient::connect(format!("http://{}", self.user_service_addr))
                .await
                .map_err(|e| {
                    error!("Failed to connect to UserService: {}", e);
                    Status::internal("Failed to connect to UserService")
                })?;

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
            let mut conn = self.redis_manager.get_write_conn().await?;
            let user_followees_key = format!("{}:followees", user_id);
            let followee_followers_key = format!("{}:followers", followee_id);

            // sets the NX option for ZADD;
            // only add a member if it doesn't already exist
            let options = redis::SortedSetAddOptions::add_only();

            let _: () = redis::pipe()
                .zadd_options(
                    user_followees_key.clone(),
                    &[(timestamp, followee_id)],
                    user_followees_key,
                    &options,
                )
                .zadd_options(
                    followee_followers_key.clone(),
                    &[(timestamp, user_id)],
                    followee_followers_key,
                    &options,
                )
                .query_async(&mut conn)
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

        // Run all updates concurrently.
        tokio::try_join!(redis_update, mongo_update_user, mongo_update_followee)?;
        Ok(())
    }

    /// Internal logic to remove a follow relationship.
    async fn _unfollow(&self, user_id: i64, followee_id: i64) -> Result<(), Status> {
        let redis_update = async {
            let mut conn = self.redis_manager.get_write_conn().await?;
            let user_followees_key = format!("{}:followees", user_id);
            let followee_followers_key = format!("{}:followers", followee_id);

            let _: () = redis::pipe()
                .zrem(user_followees_key, followee_id)
                .zrem(followee_followers_key, user_id)
                .query_async(&mut conn)
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

        let mut redis_conn = self.redis_manager.get_read_conn().await?;

        let redis_followers: Vec<i64> = redis_conn.zrange(&key, 0, -1).await.map_err(|e| {
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

        // Asynchronously update cache without blocking the response.
        if !redis_zset.is_empty() {
            let write_conn_res = self.redis_manager.get_write_conn().await;
            tokio::spawn(async move {
                if let Ok(mut conn) = write_conn_res {
                    match conn
                        .zadd_multiple::<&str, i64, i64, ()>(&key, &redis_zset)
                        .await
                    {
                        Ok(_) => info!("Updated Redis cache for followers of {}", user_id),
                        Err(e) => warn!("Failed to update Redis cache for {}: {}", user_id, e),
                    }
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

        let mut redis_conn = self.redis_manager.get_read_conn().await?;

        let redis_followees: Vec<i64> = redis_conn.zrange(&key, 0, -1).await.map_err(|e| {
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

            let user_ids: Vec<i64> = edges.iter().map(|e| e.user_id).collect();
            let zset_items: Vec<(i64, i64)> =
                edges.iter().map(|e| (e.timestamp, e.user_id)).collect();

            (user_ids, zset_items)
        } else {
            warn!("User {} not found in MongoDB", user_id);
            (Vec::new(), Vec::new())
        };

        // Asynchronously update cache.
        if !redis_zset.is_empty() {
            let write_conn_res = self.redis_manager.get_write_conn().await;
            tokio::spawn(async move {
                if let Ok(mut conn) = write_conn_res {
                    match conn
                        .zadd_multiple::<&str, i64, i64, ()>(&key, &redis_zset)
                        .await
                    {
                        Ok(_) => info!("Updated Redis cache for followees of {}", user_id),
                        Err(e) => warn!("Failed to update Redis cache for {}: {}", user_id, e),
                    }
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
