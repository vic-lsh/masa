#![allow(dead_code)]
use chrono::Utc;
use once_cell::sync::Lazy;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex as StdMutex;
use std::time::{SystemTime, UNIX_EPOCH};
// No longer need tokio::sync::Mutex for Redis
use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::{Request, Response, Status};
use tracing::{error, info, warn};

use mongodb::{
    bson::{doc, Document},
    Collection,
};

use deadpool_redis::{Connection, Pool};
// Use the redis-rs client traits and types
use deadpool_redis::redis::{AsyncCommands, RedisError};

// Custom Epoch (January 1, 2018 Midnight GMT = 2018-01-01T00:00:00Z)
const CUSTOM_EPOCH: u64 = 1_514_764_800_000;

// Import the generated gRPC code
pub mod social_network {
    tonic::include_proto!("user");
}

pub mod social_graph {
    tonic::include_proto!("social_graph");
}

use social_graph::social_graph_service_client::SocialGraphServiceClient;
use social_graph::InsertUserRequest;
use social_network::{
    user_service_server::UserService, ComposeCreatorResponse, ComposeCreatorWithUserIdRequest,
    ComposeCreatorWithUsernameRequest, Creator, GetUserIdRequest, GetUserIdResponse, LoginRequest,
    LoginResponse, RegisterUserRequest, RegisterUserResponse, RegisterUserWithIdRequest,
    RegisterUserWithIdResponse,
};

// --- Data Structures (Unchanged) ---
#[derive(Debug, Serialize, Deserialize)]
struct LoginInfo {
    password: String,
    salt: String,
    user_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    exp: usize,
    user_id: String,
    username: String,
    timestamp: String,
    ttl: String,
}

// --- Unique ID Generation Logic (Unchanged) ---
struct UserIdCounter {
    current_timestamp: i64,
    counter: i32,
}

static USER_ID_COUNTER: Lazy<StdMutex<UserIdCounter>> = Lazy::new(|| {
    StdMutex::new(UserIdCounter {
        current_timestamp: -1,
        counter: 0,
    })
});

fn get_counter(timestamp: i64) -> i32 {
    let mut state = USER_ID_COUNTER.lock().unwrap();
    if state.current_timestamp > timestamp {
        panic!("Timestamps are not incremental.");
    }
    if state.current_timestamp == timestamp {
        state.counter += 1;
    } else {
        state.current_timestamp = timestamp;
        state.counter = 0;
    }
    state.counter
}

fn generate_user_id(machine_id: &str) -> Result<i64, Status> {
    let now = SystemTime::now();
    let duration_since_epoch = now
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Status::internal(format!("System time error: {}", e)))?;

    let timestamp = duration_since_epoch.as_millis() as i64 - CUSTOM_EPOCH as i64;
    let counter = get_counter(timestamp);
    let mut timestamp_hex = format!("{:010x}", timestamp);
    if timestamp_hex.len() > 10 {
        timestamp_hex = timestamp_hex
            .split_at(timestamp_hex.len() - 10)
            .1
            .to_string();
    }
    let counter_hex = format!("{:03x}", counter);
    let id_str = format!("{}{}{}", machine_id, timestamp_hex, counter_hex);
    let user_id = i64::from_str_radix(&id_str, 16)
        .map_err(|e| Status::internal(format!("Failed to parse user_id hex: {}", e)))?;
    Ok(user_id & 0x7FFFFFFFFFFFFFFF)
}

// --- Utility Functions (Unchanged) ---
fn gen_random_string(len: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

// --- Service Implementation (Updated) ---

#[derive(Clone)]
pub struct UserServer {
    mongo_user_collection: Collection<Document>,
    // --- UPDATED ---
    // Store the pool, not a single connection
    redis_pool: Pool,
    social_graph_service_ip: String,
    social_graph_service_port: u16,
    social_graph_service_replicas: u8,
    jwt_secret: String,
    machine_id: String,
}

impl UserServer {
    /// Creates a new instance of the UserServer.
    pub fn new(
        mongo_user_collection: Collection<Document>,
        // --- UPDATED ---
        redis_pool: Pool,
        social_graph_service_ip: String,
        social_graph_service_port: u16,
        social_graph_service_replicas: u8,
        jwt_secret: String,
        machine_id: String,
    ) -> Self {
        Self {
            mongo_user_collection,
            redis_pool,
            social_graph_service_ip,
            social_graph_service_port,
            social_graph_service_replicas,
            jwt_secret,
            machine_id,
        }
    }

    /// --- NEW HELPER ---
    /// Gets a connection from the pool and maps errors to a gRPC Status.
    async fn get_redis_conn(&self) -> Result<Connection, Status> {
        self.redis_pool.get().await.map_err(|e| {
            error!("Failed to get Redis connection from pool: {}", e);
            // This error indicates the pool is unhealthy or can't connect.
            Status::internal("Cache service unavailable")
        })
    }

    /// --- NEW HELPER ---
    /// Centralizes Redis error logging for gRPC handlers.
    fn handle_redis_error(e: RedisError, cache_key: &str) -> Status {
        error!("Redis command failed for key '{}': {}", cache_key, e);
        // Don't expose specific DB errors to the client.
        Status::internal("An internal cache error occurred")
    }

    async fn social_graph_client(&self) -> SocialGraphServiceClient<LoadBalancedChannel> {
        let channel = LoadBalancedChannel::new(
            self.social_graph_service_ip.clone(),
            self.social_graph_service_port,
            self.social_graph_service_replicas,
        )
        .await;
        SocialGraphServiceClient::new(channel)
    }
}

#[tonic::async_trait]
impl UserService for UserServer {
    async fn register_user(
        &self,
        request: Request<RegisterUserRequest>,
    ) -> Result<Response<RegisterUserResponse>, Status> {
        let req = request.into_inner();
        info!("RegisterUser for username: {}", req.username);

        // Generate a new user_id
        let user_id = generate_user_id(&self.machine_id)?;

        self.register_user_internal(
            req.req_id,
            req.first_name,
            req.last_name,
            req.username,
            req.password,
            user_id,
            req.carrier,
        )
        .await?;

        Ok(Response::new(RegisterUserResponse {}))
    }

    async fn register_user_with_id(
        &self,
        request: Request<RegisterUserWithIdRequest>,
    ) -> Result<Response<RegisterUserWithIdResponse>, Status> {
        let req = request.into_inner();
        info!("RegisterUserWithId for username: {}", req.username);

        self.register_user_internal(
            req.req_id,
            req.first_name,
            req.last_name,
            req.username,
            req.password,
            req.user_id,
            req.carrier,
        )
        .await?;

        Ok(Response::new(RegisterUserWithIdResponse {}))
    }

    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        let req = request.into_inner();
        info!("Login attempt for username: {}", req.username);

        // --- UPDATED: Get connection from pool ---
        let mut conn = self.get_redis_conn().await?;
        let cache_key = format!("{}:login", &req.username);

        // 1. Try to get login info from cache
        // We use redis-rs's `get` command, which is type-safe.
        let cached_result: Result<Option<String>, RedisError> = conn.get(&cache_key).await;

        let (user_id_stored, salt_stored, password_stored) = match cached_result {
            // --- Case 1: Cache Hit ---
            Ok(Some(cached)) => {
                info!("Cache hit for user: {}", req.username);
                let login_info: LoginInfo = serde_json::from_str(&cached)
                    .map_err(|e| Status::internal(format!("Cache data corruption: {}", e)))?;
                (login_info.user_id, login_info.salt, login_info.password)
            }

            // --- Case 2: Cache Miss (or Error) ---
            Ok(None) => {
                info!("Cache miss for user: {}", req.username);
                // Go to DB
                self.login_cache_miss(&mut conn, &req.username, &cache_key)
                    .await?
            }
            Err(e) => {
                // If cache read fails, log it and proceed to DB.
                // This makes your service resilient to cache failures.
                warn!(
                    "Redis GET failed for '{}': {}. Falling back to DB.",
                    &cache_key, e
                );
                self.login_cache_miss(&mut conn, &req.username, &cache_key)
                    .await?
            }
        };

        // 4. Validate password
        let mut hasher = Sha256::new();
        hasher.update(format!("{}{}", req.password, salt_stored));
        let result = hasher.finalize();
        let password_hashed = hex::encode(result);

        if password_hashed != password_stored {
            return Err(Status::unauthenticated("Incorrect username or password"));
        }

        // 5. Generate JWT token
        let now = Utc::now().timestamp() as usize;
        let claims = Claims {
            exp: now + 3600, // Expires in 1 hour
            user_id: user_id_stored.to_string(),
            username: req.username.clone(),
            timestamp: now.to_string(),
            ttl: "3600".to_string(),
        };
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(self.jwt_secret.as_ref()),
        )
        .map_err(|e| Status::internal(format!("Failed to create token: {}", e)))?;

        Ok(Response::new(LoginResponse { token }))
    }

    // --- Other gRPC methods (Unchanged) ---

    async fn compose_creator_with_user_id(
        &self,
        request: Request<ComposeCreatorWithUserIdRequest>,
    ) -> Result<Response<ComposeCreatorResponse>, Status> {
        let req = request.into_inner();
        let creator = Creator {
            user_id: req.user_id,
            username: req.username,
        };
        Ok(Response::new(ComposeCreatorResponse {
            creator: Some(creator),
        }))
    }

    async fn compose_creator_with_username(
        &self,
        request: Request<ComposeCreatorWithUsernameRequest>,
    ) -> Result<Response<ComposeCreatorResponse>, Status> {
        let req = request.into_inner();
        let user_id = self.get_user_id_internal(&req.username).await?;
        let creator = Creator {
            user_id,
            username: req.username,
        };
        Ok(Response::new(ComposeCreatorResponse {
            creator: Some(creator),
        }))
    }

    async fn get_user_id(
        &self,
        request: Request<GetUserIdRequest>,
    ) -> Result<Response<GetUserIdResponse>, Status> {
        let req = request.into_inner();
        let user_id = self.get_user_id_internal(&req.username).await?;
        Ok(Response::new(GetUserIdResponse { user_id }))
    }
}

// --- Helper methods for the service implementation ---
impl UserServer {
    /// --- NEW HELPER ---
    /// Handles the logic for a cache miss during login.
    async fn login_cache_miss(
        &self,
        conn: &mut Connection,
        username: &str,
        cache_key: &str,
    ) -> Result<(i64, String, String), Status> {
        // 2. Query MongoDB
        let filter = doc! { "username": username };
        let user_doc = self
            .mongo_user_collection
            .find_one(filter, None)
            .await
            .map_err(|e| Status::internal(format!("Database error: {}", e)))?
            .ok_or_else(|| Status::not_found(format!("User '{}' not found", username)))?;

        let user_id_stored = user_doc.get_i64("user_id").unwrap_or(-1);
        let salt_stored = user_doc.get_str("salt").unwrap_or("").to_string();
        let password_stored = user_doc.get_str("password").unwrap_or("").to_string();

        // 3. Cache the result
        let login_info = LoginInfo {
            user_id: user_id_stored,
            salt: salt_stored.clone(),
            password: password_stored.clone(),
        };
        let login_info_str = serde_json::to_string(&login_info).unwrap();

        let _: () = conn
            .set(cache_key, login_info_str)
            .await
            .unwrap_or_else(|e| {
                warn!(
                    "Redis SET failed for '{}': {}. Proceeding without caching.",
                    cache_key, e
                );
                // Return a dummy () so the unwrap_or_else works
                ()
            });

        Ok((user_id_stored, salt_stored, password_stored))
    }

    // --- register_user_internal (Unchanged) ---
    async fn register_user_internal(
        &self,
        req_id: i64,
        first_name: String,
        last_name: String,
        username: String,
        password: String,
        user_id: i64,
        carrier: HashMap<String, String>,
    ) -> Result<(), Status> {
        // 1. Check if username already exists
        let filter = doc! { "username": &username };
        if self
            .mongo_user_collection
            .find_one(filter.clone(), None)
            .await
            .map_err(|e| Status::internal(format!("DB error: {}", e)))?
            .is_some()
        {
            warn!("Attempt to register existing username: {}", username);
            return Err(Status::already_exists(format!(
                "User '{}' already exists",
                username
            )));
        }

        // 2. Hash password
        let salt = gen_random_string(32);
        let mut hasher = Sha256::new();
        hasher.update(format!("{}{}", password, salt));
        let password_hashed = hex::encode(hasher.finalize());

        // 3. Create user document and insert into MongoDB
        let user_doc = doc! {
            "user_id": user_id,
            "first_name": first_name,
            "last_name": last_name,
            "username": &username,
            "password": password_hashed,
            "salt": salt,
        };

        self.mongo_user_collection
            .insert_one(user_doc, None)
            .await
            .map_err(|e| {
                error!("Failed to insert user into MongoDB: {}", e);
                Status::internal("Failed to register user")
            })?;

        info!(
            "Successfully registered user '{}' with user_id {}",
            &username, user_id
        );

        // 4. Register the user in social graph storage.
        let mut social_graph_client = self.social_graph_client().await;
        social_graph_client
            .insert_user(Request::new(InsertUserRequest {
                req_id,
                user_id,
                carrier,
            }))
            .await
            .map_err(|e| {
                error!("Failed to insert user {} into social graph: {}", user_id, e);
                Status::internal("Failed to register user in social graph")
            })?;

        Ok(())
    }

    // --- get_user_id_internal (UPDATED) ---
    async fn get_user_id_internal(&self, username: &str) -> Result<i64, Status> {
        // --- UPDATED: Get connection from pool ---
        let mut conn = self.get_redis_conn().await?;
        let cache_key = format!("{}:user_id", username);

        // 1. Try to get from cache
        let cached_result: Result<Option<String>, RedisError> = conn.get(&cache_key).await;

        match cached_result {
            // --- Case 1: Cache Hit ---
            Ok(Some(user_id_str)) => {
                if let Ok(user_id) = user_id_str.parse::<i64>() {
                    info!("Cache hit for user_id lookup: {}", username);
                    return Ok(user_id);
                } else {
                    warn!("Failed to parse cached user_id for {}", username);
                    // Data is corrupt, fall through to DB
                }
            }
            // --- Case 2: Cache Miss ---
            Ok(None) => {
                // Fall through to DB
            }
            // --- Case 3: Cache Error ---
            Err(e) => {
                warn!(
                    "Redis GET failed for {}: {}. Falling back to DB.",
                    &cache_key, e
                );
                // Fall through to DB
            }
        }

        // 2. If not in cache (or cache failed), get from MongoDB
        info!("Cache miss for user_id lookup: {}", username);
        let filter = doc! { "username": username };
        let user_doc = self
            .mongo_user_collection
            .find_one(filter, None)
            .await
            .map_err(|e| Status::internal(format!("Database error: {}", e)))?
            .ok_or_else(|| Status::not_found(format!("User '{}' not found", username)))?;

        let user_id = user_doc
            .get_i64("user_id")
            .map_err(|_| Status::internal("User data is corrupt: missing user_id"))?;

        // 3. Cache the result for future lookups
        // Log errors but don't fail the request
        // We also need to specify the type for the turbofish operator
        let _: () = conn
            .set(&cache_key, user_id.to_string())
            .await
            .unwrap_or_else(|e| {
                warn!(
                    "Redis SET failed for {}: {}. Proceeding without caching.",
                    &cache_key, e
                );
                ()
            });

        Ok(user_id)
    }
}
