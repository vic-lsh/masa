use std::collections::{HashMap, HashSet};
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

use deadpool_redis::redis::cluster::ClusterClient;
use deadpool_redis::redis::{Client as RedisClient, RedisError};
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::{FindOneAndUpdateOptions, FindOneOptions, ReturnDocument};
use mongodb::{Client, Collection};

use tonic::async_trait;
use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use tracing::{error, info, warn};

use post_storage::post_storage_service_client::PostStorageServiceClient;
use post_storage::ReadPostsRequest;
use user_timeline::user_timeline_service_server::{UserTimelineService, UserTimelineServiceServer};
use user_timeline::{
    ReadUserTimelineRequest, ReadUserTimelineResponse, WriteUserTimelineRequest,
    WriteUserTimelineResponse,
};

use mongodb::IndexModel;

pub mod user_timeline {
    tonic::include_proto!("user_timeline");
}

pub mod post_storage {
    tonic::include_proto!("post_storage");
}

pub mod user {
    tonic::include_proto!("user");
}

pub mod usermention {
    tonic::include_proto!("usermention");
}

pub mod media {
    tonic::include_proto!("media");
}

pub mod url_shorten {
    tonic::include_proto!("url_shorten");
}

/// Configuration required to bootstrap the user timeline gRPC server.
#[derive(Clone, Debug)]
pub struct Args {
    pub listen_addr: String,
    pub mongodb_uri: String,
    pub mongodb_database: String,
    pub mongodb_collection: String,
    pub redis_url: Option<String>,
    pub redis_primary_url: Option<String>,
    pub redis_replica_url: Option<String>,
    pub redis_cluster_urls: Option<String>,

    // --- CHANGE 2: Replace single address with IP, Port, and Replicas ---
    // Old: pub post_storage_addr: String,
    pub post_storage_ip: String,
    pub post_storage_port: u16,
    pub post_storage_replicas: u8,
}

impl Args {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            listen_addr: env::var("USER_TIMELINE_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),

            mongodb_uri: env::var("USER_TIMELINE_MONGODB_URI")
                .expect("USER_TIMELINE_MONGODB_URI must be set"),

            // --- CHANGE 3: Parse the new connection details ---
            post_storage_ip: env::var("POST_STORAGE_IP")
                .unwrap_or_else(|_| "post_storage".to_string()), // Default docker service name

            post_storage_port: env::var("POST_STORAGE_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .expect("POST_STORAGE_PORT must be a valid number"),

            post_storage_replicas: env::var("POST_STORAGE_REPLICAS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()
                .expect("POST_STORAGE_REPLICAS must be a valid number"),

            redis_url: env::var("USER_TIMELINE_REDIS_URL").ok(),
            redis_primary_url: env::var("USER_TIMELINE_REDIS_PRIMARY_URL").ok(),
            redis_replica_url: env::var("USER_TIMELINE_REDIS_REPLICA_URL").ok(),
            redis_cluster_urls: env::var("USER_TIMELINE_REDIS_CLUSTER_URLS").ok(),

            mongodb_database: env::var("USER_TIMELINE_MONGODB_DATABASE")
                .unwrap_or_else(|_| "user-timeline".to_string()),
            mongodb_collection: env::var("USER_TIMELINE_MONGODB_COLLECTION")
                .unwrap_or_else(|_| "user-timeline".to_string()),
        })
    }
}

#[derive(Clone)]
enum RedisBackend {
    Standalone(Arc<RedisClient>),
    PrimaryReplica {
        primary: Arc<RedisClient>,
        replica: Arc<RedisClient>,
    },
    Cluster(Arc<ClusterClient>),
}

impl RedisBackend {
    fn from_args(args: &Args) -> Result<Self, Box<dyn std::error::Error>> {
        if let Some(cluster_urls) = args.redis_cluster_urls.as_ref() {
            let nodes: Vec<String> = cluster_urls
                .split(',')
                .filter_map(|entry| {
                    let trimmed = entry.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
                .collect();
            if nodes.is_empty() {
                return Err(
                    "USER_TIMELINE_REDIS_CLUSTER_URLS must provide at least one node".into(),
                );
            }
            let client = ClusterClient::new(nodes)?;
            return Ok(RedisBackend::Cluster(Arc::new(client)));
        }

        match (&args.redis_primary_url, &args.redis_replica_url, &args.redis_url) {
            (Some(primary), Some(replica), _) => {
                let primary_client = RedisClient::open(primary.as_str())?;
                let replica_client = RedisClient::open(replica.as_str())?;
                Ok(RedisBackend::PrimaryReplica {
                    primary: Arc::new(primary_client),
                    replica: Arc::new(replica_client),
                })
            }
            (_, _, Some(url)) => {
                let client = RedisClient::open(url.as_str())?;
                Ok(RedisBackend::Standalone(Arc::new(client)))
            }
            _ => Err("Redis configuration missing: provide USER_TIMELINE_REDIS_URL or primary/replica URLs".into()),
        }
    }

    async fn zadd_nx(&self, key: &str, member: &str, score: f64) -> Result<(), Status> {
        match self {
            RedisBackend::Standalone(client) => zadd_nx(client, key, member, score).await,
            RedisBackend::PrimaryReplica { primary, .. } => {
                zadd_nx(primary, key, member, score).await
            }
            RedisBackend::Cluster(client) => zadd_nx_cluster(client, key, member, score).await,
        }
    }

    async fn zadd_all(&self, key: &str, entries: &[(String, f64)]) -> Result<(), Status> {
        if entries.is_empty() {
            return Ok(());
        }
        match self {
            RedisBackend::Standalone(client) => zadd_all(client, key, entries).await,
            RedisBackend::PrimaryReplica { primary, .. } => zadd_all(primary, key, entries).await,
            RedisBackend::Cluster(client) => zadd_all_cluster(client, key, entries).await,
        }
    }

    async fn zrevrange(&self, key: &str, start: i64, stop: i64) -> Result<Vec<String>, Status> {
        match self {
            RedisBackend::Standalone(client) => zrevrange(client, key, start, stop).await,
            RedisBackend::PrimaryReplica { replica, .. } => {
                zrevrange(replica, key, start, stop).await
            }
            RedisBackend::Cluster(client) => zrevrange_cluster(client, key, start, stop).await,
        }
    }
}
// ... [End of Redis helper functions] ...
async fn zadd_nx(client: &RedisClient, key: &str, member: &str, score: f64) -> Result<(), Status> {
    let mut conn = client
        .get_multiplexed_tokio_connection()
        .await
        .map_err(redis_to_status)?;
    let mut command = deadpool_redis::redis::cmd("ZADD");
    command.arg(key).arg("NX").arg(score).arg(member);
    command
        .query_async(&mut conn)
        .await
        .map_err(redis_to_status)
}
async fn zadd_all(
    client: &RedisClient,
    key: &str,
    entries: &[(String, f64)],
) -> Result<(), Status> {
    let mut conn = client
        .get_multiplexed_tokio_connection()
        .await
        .map_err(redis_to_status)?;
    let mut command = deadpool_redis::redis::cmd("ZADD");
    command.arg(key);
    for (member, score) in entries {
        command.arg(*score).arg(member);
    }
    command
        .query_async(&mut conn)
        .await
        .map_err(redis_to_status)
}
async fn zrevrange(
    client: &RedisClient,
    key: &str,
    start: i64,
    stop: i64,
) -> Result<Vec<String>, Status> {
    let mut conn = client
        .get_multiplexed_tokio_connection()
        .await
        .map_err(redis_to_status)?;
    deadpool_redis::redis::cmd("ZREVRANGE")
        .arg(key)
        .arg(start)
        .arg(stop)
        .query_async(&mut conn)
        .await
        .map_err(redis_to_status)
}
async fn zadd_nx_cluster(
    client: &ClusterClient,
    key: &str,
    member: &str,
    score: f64,
) -> Result<(), Status> {
    let mut conn = client
        .get_async_connection()
        .await
        .map_err(redis_to_status)?;
    let mut command = deadpool_redis::redis::cmd("ZADD");
    command.arg(key).arg("NX").arg(score).arg(member);
    command
        .query_async(&mut conn)
        .await
        .map_err(redis_to_status)
}
async fn zadd_all_cluster(
    client: &ClusterClient,
    key: &str,
    entries: &[(String, f64)],
) -> Result<(), Status> {
    let mut conn = client
        .get_async_connection()
        .await
        .map_err(redis_to_status)?;
    let mut command = deadpool_redis::redis::cmd("ZADD");
    command.arg(key);
    for (member, score) in entries {
        command.arg(*score).arg(member);
    }
    command
        .query_async(&mut conn)
        .await
        .map_err(redis_to_status)
}
async fn zrevrange_cluster(
    client: &ClusterClient,
    key: &str,
    start: i64,
    stop: i64,
) -> Result<Vec<String>, Status> {
    let mut conn = client
        .get_async_connection()
        .await
        .map_err(redis_to_status)?;
    deadpool_redis::redis::cmd("ZREVRANGE")
        .arg(key)
        .arg(start)
        .arg(stop)
        .query_async(&mut conn)
        .await
        .map_err(redis_to_status)
}
fn redis_to_status(err: RedisError) -> Status {
    Status::internal(format!("Redis error: {}", err))
}

#[derive(Clone)]
pub struct UserTimelineServiceImpl {
    collection: Collection<Document>,
    redis: RedisBackend,
    // --- CHANGE 4: Use LoadBalancedChannel instead of generic Channel ---
    post_storage_client: PostStorageServiceClient<LoadBalancedChannel>,
}

impl UserTimelineServiceImpl {
    pub async fn new(args: Args) -> Result<Self, Box<dyn std::error::Error>> {
        let mongo_client = Client::with_uri_str(&args.mongodb_uri).await?;
        let collection = mongo_client
            .database(&args.mongodb_database)
            .collection::<Document>(&args.mongodb_collection);

        let index_model = IndexModel::builder().keys(doc! { "user_id": 1 }).build();

        // This will only create the index if it doesn't exist.
        collection.create_index(index_model, None).await?;
        info!("Successfully ensured index on 'user_id' exists.");

        let redis = RedisBackend::from_args(&args)?;

        // --- CHANGE 5: Initialize using LoadBalancedChannel ---
        let channel = LoadBalancedChannel::new(
            args.post_storage_ip.clone(),
            args.post_storage_port,
            args.post_storage_replicas as u8,
        )
        .await;

        let post_storage_client = PostStorageServiceClient::new(channel);

        Ok(Self {
            collection,
            redis,
            post_storage_client,
        })
    }
}

#[async_trait]
impl UserTimelineService for UserTimelineServiceImpl {
    async fn write_user_timeline(
        &self,
        request: Request<WriteUserTimelineRequest>,
    ) -> Result<Response<WriteUserTimelineResponse>, Status> {
        let req = request.into_inner();
        info!(
            "write_user_timeline: user_id={}, post_id={}, timestamp={}",
            req.user_id, req.post_id, req.timestamp
        );

        let filter = doc! { "user_id": req.user_id };
        let update = doc! {
            "$push": {
                "posts": {
                    "$each": [ { "post_id": req.post_id, "timestamp": req.timestamp } ],
                    "$position": 0,
                }
            }
        };

        let options = FindOneAndUpdateOptions::builder()
            .upsert(Some(true))
            .return_document(Some(ReturnDocument::After))
            .build();

        if let Err(err) = self
            .collection
            .find_one_and_update(filter.clone(), update.clone(), options)
            .await
        {
            warn!(
                "Mongo upsert failed for user {}: {}. Retrying without upsert.",
                req.user_id, err
            );
            let retry_options = FindOneAndUpdateOptions::builder()
                .upsert(Some(false))
                .return_document(Some(ReturnDocument::After))
                .build();
            self.collection
                .find_one_and_update(filter, update, retry_options)
                .await
                .map_err(|retry_err| {
                    error!(
                        "Mongo update failed for user {}: {}",
                        req.user_id, retry_err
                    );
                    Status::internal(format!(
                        "Failed to update MongoDB for user {}: {}",
                        req.user_id, retry_err
                    ))
                })?;
        }

        self.redis
            .zadd_nx(
                &req.user_id.to_string(),
                &req.post_id.to_string(),
                req.timestamp as f64,
            )
            .await?;

        Ok(Response::new(WriteUserTimelineResponse {}))
    }

    async fn read_user_timeline(
        &self,
        request: Request<ReadUserTimelineRequest>,
    ) -> Result<Response<ReadUserTimelineResponse>, Status> {
        let req = request.into_inner();
        info!(
            "read_user_timeline: user_id={}, start={}, stop={}",
            req.user_id, req.start, req.stop
        );

        if req.stop <= req.start || req.start < 0 {
            return Ok(Response::new(ReadUserTimelineResponse { posts: vec![] }));
        }

        let redis_stop = req.stop - 1;
        let mut post_ids: Vec<i64> = Vec::new();
        let mut seen_posts: HashSet<i64> = HashSet::new();

        let redis_ids = self
            .redis
            .zrevrange(
                &req.user_id.to_string(),
                req.start as i64,
                redis_stop as i64,
            )
            .await?;

        for post_id_str in redis_ids {
            match post_id_str.parse::<i64>() {
                Ok(post_id) => {
                    seen_posts.insert(post_id);
                    post_ids.push(post_id);
                }
                Err(err) => warn!("Invalid post id '{}' from redis: {}", post_id_str, err),
            }
        }

        let mut redis_update_map: HashMap<String, f64> = HashMap::new();
        let mongo_start = req.start as usize + post_ids.len();
        if mongo_start < req.stop as usize {
            let projection = doc! {
                "posts": { "$slice": [0, req.stop] }
            };
            let options = FindOneOptions::builder().projection(projection).build();
            if let Some(doc) = self
                .collection
                .find_one(doc! { "user_id": req.user_id }, options)
                .await
                .map_err(|err| {
                    error!("Mongo find failed for user {}: {}", req.user_id, err);
                    Status::internal(format!(
                        "Failed to load user timeline from MongoDB: {}",
                        err
                    ))
                })?
            {
                if let Some(Bson::Array(posts)) = doc.get("posts") {
                    for (idx, entry) in posts.iter().enumerate() {
                        let Some(post_doc) = entry.as_document() else {
                            continue;
                        };
                        let Ok(post_id) = post_doc.get_i64("post_id") else {
                            continue;
                        };
                        let Ok(timestamp) = post_doc.get_i64("timestamp") else {
                            continue;
                        };
                        redis_update_map.insert(post_id.to_string(), timestamp as f64);
                        if idx >= mongo_start && seen_posts.insert(post_id) {
                            post_ids.push(post_id);
                        }
                    }
                }
            }
        }

        if !redis_update_map.is_empty() {
            let updates: Vec<(String, f64)> = redis_update_map.into_iter().collect();
            self.redis
                .zadd_all(&req.user_id.to_string(), &updates)
                .await?;
        }

        if post_ids.is_empty() {
            return Ok(Response::new(ReadUserTimelineResponse { posts: vec![] }));
        }

        let mut client = self.post_storage_client.clone();
        let response = client
            .read_posts(Request::new(ReadPostsRequest {
                req_id: req.req_id,
                post_ids,
                carrier: req.carrier,
            }))
            .await
            .map_err(|err| {
                error!("post-storage ReadPosts failed: {}", err);
                Status::internal(format!("Failed to fetch posts: {}", err))
            })?;

        Ok(Response::new(ReadUserTimelineResponse {
            posts: response.into_inner().posts,
        }))
    }
}

pub async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let listen_addr: SocketAddr = args.listen_addr.parse()?;
    let service = UserTimelineServiceImpl::new(args).await?;
    info!("UserTimelineService listening on {}", listen_addr);

    Server::builder()
        .add_service(UserTimelineServiceServer::new(service))
        .serve_with_masa(listen_addr)
        .await?;

    Ok(())
}
