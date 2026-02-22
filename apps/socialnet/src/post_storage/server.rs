use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;

use async_memcached::AsciiProtocol;
use async_memcached::Client as McClient;
use futures::stream::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson};
use mongodb::options::ClientOptions;
use mongodb::{Client as MongoClient, Collection};
use tokio::sync::Mutex;
use tonic::async_trait;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use crate::media::Media as MediaProto;
use crate::post_storage::post_storage_service_server::{
    PostStorageService, PostStorageServiceServer,
};
use crate::post_storage::{
    ReadPostRequest, ReadPostResponse, ReadPostsRequest, ReadPostsResponse, StorePostRequest,
    StorePostResponse,
};
use crate::url_shorten::Url as UrlProto;
use crate::user::Creator as CreatorProto;
use crate::user_timeline::Post as PostProto;
use crate::usermention::UserMention as UserMentionProto;

#[derive(Clone, Debug)]
pub struct Args {
    /// gRPC listen address for the PostStorage service, e.g. `0.0.0.0:50065`.
    pub listen_addr: String,
    /// MongoDB connection string that stores posts.
    pub mongodb_uri: String,
    /// MongoDB database name used to persist posts.
    pub mongodb_database: String,
    /// MongoDB collection name used to persist posts.
    pub mongodb_collection: String,
    /// Memcached endpoint (ASCII protocol) used for caching post documents.
    pub memcached_addr: String,
    /// Optional time-to-live for cached posts in seconds; `None` leaves entries without an expiry.
    pub memcached_ttl_seconds: Option<u32>,
}

impl Args {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let listen_addr = std::env::var("POST_STORAGE_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string());
        let mongodb_uri = std::env::var("POST_STORAGE_MONGODB_URI")
            .unwrap_or_else(|_| "mongodb://127.0.0.1:27017".to_string());
        let mongodb_database = std::env::var("POST_STORAGE_MONGODB_DATABASE")
            .unwrap_or_else(|_| "post-db".to_string());
        let mongodb_collection =
            std::env::var("POST_STORAGE_MONGODB_COLLECTION").unwrap_or_else(|_| "post".to_string());
        let memcached_addr = std::env::var("POST_STORAGE_MEMCACHED_ADDR")
            .unwrap_or_else(|_| "tcp://127.0.0.1:11211".to_string());
        let memcached_ttl_seconds = std::env::var("POST_STORAGE_MEMCACHED_TTL")
            .ok()
            .and_then(|value| value.parse::<u32>().ok());

        Ok(Self {
            listen_addr,
            mongodb_uri,
            mongodb_database,
            mongodb_collection,
            memcached_addr,
            memcached_ttl_seconds,
        })
    }
}

#[derive(Clone)]
pub struct PostStorageServiceImpl {
    mongo_collection: Collection<PostRecord>,
    memcached: Arc<Mutex<McClient>>,
    cache_ttl: Option<i64>,
}

impl PostStorageServiceImpl {
    pub async fn new(args: &Args) -> Result<Self, Box<dyn std::error::Error>> {
        let mongo_opts = ClientOptions::parse(&args.mongodb_uri).await?;
        let mongo_client = MongoClient::with_options(mongo_opts)?;
        let mongo_collection = mongo_client
            .database(&args.mongodb_database)
            .collection::<PostRecord>(&args.mongodb_collection);

        let memcached_client = McClient::new(&args.memcached_addr).await?;
        let cache_ttl = args.memcached_ttl_seconds.map(|ttl| ttl as i64);

        Ok(Self {
            mongo_collection,
            memcached: Arc::new(Mutex::new(memcached_client)),
            cache_ttl,
        })
    }

    fn ttl(&self) -> Option<i64> {
        self.cache_ttl
    }

    fn post_to_record(post: &PostProto) -> PostRecord {
        let creator = post
            .creator
            .as_ref()
            .expect("caller ensures creator is present");
        PostRecord {
            id: None,
            post_id: post.post_id,
            req_id: post.req_id,
            text: post.text.clone(),
            timestamp: post.timestamp,
            post_type: post.post_type,
            creator: CreatorDocument {
                user_id: creator.user_id,
                username: creator.username.clone(),
            },
            user_mentions: post
                .user_mentions
                .iter()
                .map(|mention| UserMentionDocument {
                    user_id: mention.user_id as i64,
                    username: mention.username.clone(),
                })
                .collect(),
            media: post
                .media
                .iter()
                .map(|media| MediaDocument {
                    media_id: media.media_id,
                    media_type: media.media_type.clone(),
                })
                .collect(),
            urls: post
                .urls
                .iter()
                .map(|url| UrlDocument {
                    shortened_url: url.shortened_url.clone(),
                    expanded_url: url.expanded_url.clone(),
                })
                .collect(),
        }
    }

    fn record_to_post(record: &PostRecord) -> PostProto {
        PostProto {
            post_id: record.post_id,
            req_id: record.req_id,
            text: record.text.clone(),
            timestamp: record.timestamp,
            post_type: record.post_type,
            creator: Some(CreatorProto {
                user_id: record.creator.user_id,
                username: record.creator.username.clone(),
            }),
            user_mentions: record
                .user_mentions
                .iter()
                .map(|mention| UserMentionProto {
                    user_id: mention.user_id as u64,
                    username: mention.username.clone(),
                })
                .collect(),
            media: record
                .media
                .iter()
                .map(|media| MediaProto {
                    media_id: media.media_id,
                    media_type: media.media_type.clone(),
                })
                .collect(),
            urls: record
                .urls
                .iter()
                .map(|url| UrlProto {
                    shortened_url: url.shortened_url.clone(),
                    expanded_url: url.expanded_url.clone(),
                })
                .collect(),
        }
    }

    fn record_to_cache_entry(record: &PostRecord) -> CachedPost {
        CachedPost {
            post_id: record.post_id,
            req_id: record.req_id,
            text: record.text.clone(),
            timestamp: record.timestamp,
            post_type: record.post_type,
            creator: record.creator.clone(),
            user_mentions: record.user_mentions.clone(),
            media: record.media.clone(),
            urls: record.urls.clone(),
        }
    }

    fn post_from_cache(cache: &CachedPost) -> PostProto {
        PostProto {
            post_id: cache.post_id,
            req_id: cache.req_id,
            text: cache.text.clone(),
            timestamp: cache.timestamp,
            post_type: cache.post_type,
            creator: Some(CreatorProto {
                user_id: cache.creator.user_id,
                username: cache.creator.username.clone(),
            }),
            user_mentions: cache
                .user_mentions
                .iter()
                .map(|mention| UserMentionProto {
                    user_id: mention.user_id as u64,
                    username: mention.username.clone(),
                })
                .collect(),
            media: cache
                .media
                .iter()
                .map(|media| MediaProto {
                    media_id: media.media_id,
                    media_type: media.media_type.clone(),
                })
                .collect(),
            urls: cache
                .urls
                .iter()
                .map(|url| UrlProto {
                    shortened_url: url.shortened_url.clone(),
                    expanded_url: url.expanded_url.clone(),
                })
                .collect(),
        }
    }

    async fn fetch_from_mongo(&self, post_id: i64) -> Result<Option<PostRecord>, Status> {
        let filter = doc! { "post_id": post_id };
        self.mongo_collection
            .find_one(filter, None)
            .await
            .map_err(|err| {
                Status::internal(format!(
                    "MongoDB find_one failed for post {}: {}",
                    post_id, err
                ))
            })
    }

    async fn write_cache(&self, key: &str, cache_entry: &CachedPost) {
        let mut client = self.memcached.lock().await;
        match serde_json::to_string(cache_entry) {
            Ok(payload) => {
                if let Err(err) = client.set(key, &payload, self.ttl(), None).await {
                    log::warn!("Failed to set post {} in memcached: {}", key, err);
                }
            }
            Err(err) => {
                log::warn!("Failed to serialize post {} for cache: {}", key, err);
            }
        }
    }
}

#[async_trait]
impl PostStorageService for PostStorageServiceImpl {
    async fn store_post(
        &self,
        request: Request<StorePostRequest>,
    ) -> Result<Response<StorePostResponse>, Status> {
        let req = request.into_inner();
        let post = req
            .post
            .ok_or_else(|| Status::invalid_argument("post is required"))?;
        if post.creator.is_none() {
            return Err(Status::invalid_argument("post.creator is required"));
        }
        let record = Self::post_to_record(&post);

        self.mongo_collection
            .insert_one(record, None)
            .await
            .map_err(|err| Status::internal(format!("MongoDB insert failed: {}", err)))?;

        Ok(Response::new(StorePostResponse {}))
    }

    async fn read_post(
        &self,
        request: Request<ReadPostRequest>,
    ) -> Result<Response<ReadPostResponse>, Status> {
        let req = request.into_inner();
        let key = req.post_id.to_string();

        if let Some(post) = self.try_read_from_cache(&key).await? {
            return Ok(Response::new(ReadPostResponse { post: Some(post) }));
        }

        let record = self
            .fetch_from_mongo(req.post_id)
            .await?
            .ok_or_else(|| Status::not_found(format!("post {} not found", req.post_id)))?;
        let cache_entry = Self::record_to_cache_entry(&record);
        let post = Self::record_to_post(&record);

        self.write_cache(&key, &cache_entry).await;

        Ok(Response::new(ReadPostResponse { post: Some(post) }))
    }

    async fn read_posts(
        &self,
        request: Request<ReadPostsRequest>,
    ) -> Result<Response<ReadPostsResponse>, Status> {
        let req = request.into_inner();
        if req.post_ids.is_empty() {
            return Ok(Response::new(ReadPostsResponse { posts: vec![] }));
        }

        let mut unique_ids = HashSet::new();
        for post_id in &req.post_ids {
            if !unique_ids.insert(*post_id) {
                return Err(Status::invalid_argument("post_ids contain duplicates"));
            }
        }

        let mut posts_by_id: HashMap<i64, PostProto> = HashMap::new();
        let mut missing_ids: Vec<i64> = Vec::new();

        let cache_hits = self.multi_get_from_cache(&req.post_ids).await?;
        for (post_id, post) in cache_hits {
            posts_by_id.insert(post_id, post);
        }

        for post_id in &req.post_ids {
            if !posts_by_id.contains_key(post_id) {
                missing_ids.push(*post_id);
            }
        }

        if !missing_ids.is_empty() {
            let mongo_posts = self.fetch_many_from_mongo(&missing_ids).await?;
            for record in mongo_posts {
                let post = Self::record_to_post(&record);
                let cache_entry = Self::record_to_cache_entry(&record);
                let key = record.post_id.to_string();
                self.write_cache(&key, &cache_entry).await;
                posts_by_id.insert(record.post_id, post);
            }
        }

        if posts_by_id.len() != req.post_ids.len() {
            return Err(Status::not_found("one or more posts are missing"));
        }

        let ordered_posts = req
            .post_ids
            .iter()
            .map(|post_id| posts_by_id.get(post_id).cloned().unwrap())
            .collect();

        Ok(Response::new(ReadPostsResponse {
            posts: ordered_posts,
        }))
    }
}

impl PostStorageServiceImpl {
    async fn try_read_from_cache(&self, key: &str) -> Result<Option<PostProto>, Status> {
        let mut client = self.memcached.lock().await;
        match client.get(key).await {
            Ok(Some(value)) => {
                let data = value
                    .data
                    .ok_or_else(|| Status::data_loss("memcached entry missing payload for post"))?;
                let cached_post: CachedPost = serde_json::from_slice(&data).map_err(|err| {
                    Status::internal(format!("Failed to deserialize cache entry: {}", err))
                })?;
                Ok(Some(Self::post_from_cache(&cached_post)))
            }
            Ok(None) => Ok(None),
            Err(err) => Err(Status::internal(format!("Memcached get failed: {}", err))),
        }
    }

    async fn multi_get_from_cache(
        &self,
        post_ids: &[i64],
    ) -> Result<HashMap<i64, PostProto>, Status> {
        let keys: Vec<String> = post_ids.iter().map(|id| id.to_string()).collect();
        let mut client = self.memcached.lock().await;
        match client.get_multi(keys.clone()).await {
            Ok(values) => {
                let mut result = HashMap::new();
                for value in values {
                    if let Some(data) = value.data {
                        let cached_post: CachedPost =
                            serde_json::from_slice(&data).map_err(|err| {
                                Status::internal(format!(
                                    "Failed to deserialize cached post {}: {}",
                                    String::from_utf8_lossy(&value.key),
                                    err
                                ))
                            })?;
                        let post_id = cached_post.post_id;
                        result.insert(post_id, Self::post_from_cache(&cached_post));
                    }
                }
                Ok(result)
            }
            Err(err) => Err(Status::internal(format!("Memcached mget failed: {}", err))),
        }
    }

    async fn fetch_many_from_mongo(&self, post_ids: &[i64]) -> Result<Vec<PostRecord>, Status> {
        let ids: Vec<Bson> = post_ids.iter().map(|id| Bson::Int64(*id)).collect();
        let filter = doc! { "post_id": { "$in": ids } };
        let mut cursor = self
            .mongo_collection
            .find(filter, None)
            .await
            .map_err(|err| Status::internal(format!("MongoDB query failed: {}", err)))?;

        let mut records = Vec::new();
        while let Some(record) = cursor
            .try_next()
            .await
            .map_err(|err| Status::internal(format!("MongoDB cursor error: {}", err)))?
        {
            records.push(record);
        }
        Ok(records)
    }
}

pub async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    println!("creating post storage service...");
    let service_impl = PostStorageServiceImpl::new(&args).await?;
    let listen_addr: SocketAddr = args.listen_addr.parse()?;
    log::info!("PostStorageService listening on {}", listen_addr);
    println!("PostStorageService listening on {}", listen_addr);

    Server::builder()
        .add_service(PostStorageServiceServer::new(service_impl))
        .serve_with_masa(listen_addr)
        .await?;

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PostRecord {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    id: Option<ObjectId>,
    post_id: i64,
    req_id: i64,
    text: String,
    timestamp: i64,
    post_type: i32,
    creator: CreatorDocument,
    user_mentions: Vec<UserMentionDocument>,
    media: Vec<MediaDocument>,
    urls: Vec<UrlDocument>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CreatorDocument {
    user_id: i64,
    username: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct UserMentionDocument {
    user_id: i64,
    username: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct MediaDocument {
    media_id: i64,
    media_type: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct UrlDocument {
    shortened_url: String,
    expanded_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedPost {
    post_id: i64,
    req_id: i64,
    text: String,
    timestamp: i64,
    post_type: i32,
    creator: CreatorDocument,
    user_mentions: Vec<UserMentionDocument>,
    media: Vec<MediaDocument>,
    urls: Vec<UrlDocument>,
}
