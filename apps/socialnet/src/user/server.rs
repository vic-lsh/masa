#![allow(dead_code)]
use chrono::Utc;
use once_cell::sync::Lazy;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use tracing::{error, info, warn};

use mongodb::{
    bson::{doc, Document},
    Collection,
};
use redis_async::{resp::FromResp, resp_array};

// Custom Epoch (January 1, 2018 Midnight GMT = 2018-01-01T00:00:00Z)
const CUSTOM_EPOCH: u64 = 1_514_764_800_000;

// Import the generated gRPC code
pub mod social_network {
    tonic::include_proto!("user");
}

use social_network::{
    user_service_server::UserService, ComposeCreatorResponse, ComposeCreatorWithUserIdRequest,
    ComposeCreatorWithUsernameRequest, Creator, GetUserIdRequest, GetUserIdResponse, LoginRequest,
    LoginResponse, RegisterUserRequest, RegisterUserResponse, RegisterUserWithIdRequest,
    RegisterUserWithIdResponse,
};

// --- Data Structures ---

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

// --- Unique ID Generation Logic ---

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
        // This should not happen in a real-world scenario with synchronized clocks
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

    // Ensure the ID is a positive signed 64-bit integer
    Ok(user_id & 0x7FFFFFFFFFFFFFFF)
}

// --- Utility Functions ---

fn gen_random_string(len: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

// --- Service Implementation ---

#[derive(Clone)]
pub struct UserServer {
    mongo_user_collection: Collection<Document>,
    redis_conn: Arc<Mutex<redis_async::client::PairedConnection>>,
    jwt_secret: String,
    machine_id: String,
    // In a real application, a client pool for SocialGraphService would be here.
    // social_graph_client_pool: Arc<SomeClientPool>,
}

impl UserServer {
    /// Creates a new instance of the UserServer.
    pub fn new(
        mongo_user_collection: Collection<Document>,
        redis_conn: Arc<Mutex<redis_async::client::PairedConnection>>,
        jwt_secret: String,
        machine_id: String,
    ) -> Self {
        Self {
            mongo_user_collection,
            redis_conn,
            jwt_secret,
            machine_id,
        }
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
            req.first_name,
            req.last_name,
            req.username,
            req.password,
            user_id,
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
            req.first_name,
            req.last_name,
            req.username,
            req.password,
            req.user_id,
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

        let redis_conn = &mut *self.redis_conn.lock().await;

        let cache_key = format!("{}:login", &req.username);

        // 1. Try to get login info from cache
        let get_cmd = resp_array!["GET", &cache_key];
        let cached_val = redis_conn.send(get_cmd).await.map_err(|e| {
            error!("Redis GET failed for '{}': {}", &cache_key, e);
            Status::internal("Cache service unavailable")
        })?;
        let cached_login: Option<String> = Option::from_resp(cached_val)
            .map_err(|e| Status::internal(format!("Cache data corruption: {}", e)))?;

        let (user_id_stored, salt_stored, password_stored) = if let Some(cached) = cached_login {
            info!("Cache hit for user: {}", req.username);
            let login_info: LoginInfo = serde_json::from_str(&cached)
                .map_err(|e| Status::internal(format!("Cache data corruption: {}", e)))?;
            (login_info.user_id, login_info.salt, login_info.password)
        } else {
            // 2. If not in cache, query MongoDB
            info!("Cache miss for user: {}", req.username);
            let filter = doc! { "username": &req.username };
            let user_doc = self
                .mongo_user_collection
                .find_one(filter, None)
                .await
                .map_err(|e| Status::internal(format!("Database error: {}", e)))?
                .ok_or_else(|| Status::not_found(format!("User '{}' not found", req.username)))?;

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
            let set_cmd = resp_array!["SET", &cache_key, login_info_str];
            let _: redis_async::resp::RespValue = redis_conn.send(set_cmd).await.map_err(|e| {
                error!("Redis SET failed for '{}': {}", &cache_key, e);
                Status::internal("Cache service unavailable")
            })?;

            (user_id_stored, salt_stored, password_stored)
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
            username: req.username,
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
    async fn register_user_internal(
        &self,
        first_name: String,
        last_name: String,
        username: String,
        password: String,
        user_id: i64,
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

        // 4. Call SocialGraphService to insert user
        // In a real application, you would make a gRPC call here.
        // For this example, we'll just log it.
        info!(
            "(Mock) Calling SocialGraphService to insert user_id: {}",
            user_id
        );
        // let mut social_graph_client = self.social_graph_client_pool.get().await?;
        // social_graph_client.insert_user(InsertUserRequest { ... }).await?;

        Ok(())
    }

    async fn get_user_id_internal(&self, username: &str) -> Result<i64, Status> {
        let redis_conn = &mut *self.redis_conn.lock().await;

        let cache_key = format!("{}:user_id", username);

        // 1. Try to get from cache
        let get_cmd = resp_array!["GET", &cache_key];
        let cached_val = redis_conn.send(get_cmd).await.map_err(|e| {
            error!("Redis GET failed for {}: {}", &cache_key, e);
            Status::internal("Cache service unavailable")
        })?;

        if let Ok(Some(user_id_str)) = <Option<String>>::from_resp(cached_val) {
            if let Ok(user_id) = user_id_str.parse::<i64>() {
                info!("Cache hit for user_id lookup: {}", username);
                return Ok(user_id);
            } else {
                warn!("Failed to parse cached user_id for {}", username);
            }
        }

        // 2. If not in cache, get from MongoDB
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
        let set_cmd = resp_array!["SET", &cache_key, user_id.to_string()];
        let _: redis_async::resp::RespValue = redis_conn.send(set_cmd).await.map_err(|e| {
            error!("Redis SET failed for {}: {}", &cache_key, e);
            Status::internal("Cache service unavailable")
        })?;

        Ok(user_id)
    }
}
