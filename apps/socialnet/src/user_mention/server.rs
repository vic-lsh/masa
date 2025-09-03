use app_utils::pool::McPool;
use async_memcached::AsciiProtocol;
use futures::StreamExt;
use mongodb::bson::{doc, Bson};
use mongodb::Client as MongoClient;
use std::collections::HashSet;
use std::error::Error;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::{error, info};

use user_mention_service::{
    user_mention_service_server::{UserMentionService, UserMentionServiceServer},
    ComposeUserMentionRequest, ComposeUserMentionResponse, ErrorCode, ServiceException,
    UserMention,
};

pub mod user_mention_service {
    tonic::include_proto!("usermention");
}

pub struct UserMentionServiceImpl {
    mc_pool: Arc<McPool>,
    mongo_client: Arc<MongoClient>,
}

use crate::user_mention::db::{initialize_database, initialize_memcached_pool, UserMentionStruct};

impl UserMentionServiceImpl {
    pub async fn new() -> Result<Self, Box<dyn Error>> {
        let mongo_client = match initialize_database().await {
            Ok(client) => client,
            Err(e) => {
                eprintln!("Failed to initialize MongoDB: {:?}", e);
                return Err(e);
            }
        };

        let mc_pool = match initialize_memcached_pool().await {
            Ok(pool) => pool,
            Err(e) => {
                eprintln!("Failed to initialize Memcached: {:?}", e);
                return Err(e);
            }
        };
        Ok(Self {
            mc_pool: Arc::new(mc_pool),
            mongo_client: Arc::new(mongo_client),
        })
    }
}

#[tonic::async_trait]
impl UserMentionService for UserMentionServiceImpl {
    async fn compose_user_mentions(
        &self,
        request: Request<ComposeUserMentionRequest>,
    ) -> Result<Response<ComposeUserMentionResponse>, Status> {
        info!("Got a request: {:?}", request);

        let req = request.into_inner();

        // Return if the request is empty
        if req.usernames.is_empty() {
            return Ok(Response::new(ComposeUserMentionResponse {
                user_mentions: vec![],
                exception: None,
            }));
        }

        let mut user_mentions = Vec::new();
        let mut exception = None;
        let mut missing_keys: HashSet<String> = req.usernames.iter().cloned().collect();

        // Find user mentions in memcached
        let mut mc_client = self.mc_pool.get().await;
        let mc_resp = mc_client.get_multi(req.usernames.clone()).await;

        match mc_resp {
            Err(e) if e.to_string().contains("NotFound") => {
                error!("Keys not found in memcached: {:?}", e);
            }
            Ok(entries) => {
                for entry in entries {
                    if let Ok(usr_id) = String::from_utf8(entry.data.expect("data must exist")) {
                        let username = String::from_utf8(entry.key).unwrap_or_default();
                        missing_keys.remove(&username);

                        let username_md = if cfg!(debug_assertions) {
                            format!("{}@memcached", username)
                        } else {
                            username
                        };
                        user_mentions.push(UserMention {
                            username: username_md,
                            user_id: usr_id.parse().unwrap_or_default(),
                        });
                    }
                }
            }
            Err(e) => {
                error!("Error fetching from memcached: {:?}", e);
                exception = Some(ServiceException {
                    error_code: ErrorCode::Unknown as i32,
                    message: format!("Memcached error: {}", e),
                });
            }
        }

        // Continue to MongoDB if not found in memcached
        if !missing_keys.is_empty() {
            let collection = self
                .mongo_client
                .database("usermention-db")
                .collection::<UserMentionStruct>("usermention");
            let in_array: Vec<Bson> = missing_keys.iter().cloned().map(Bson::String).collect();
            let filter = doc! { "username": { "$in": Bson::Array(in_array) } };
            let cursor = collection.find(filter, None).await;

            if let Ok(mut cursor) = cursor {
                while let Some(doc) = cursor.next().await {
                    match doc {
                        Ok(user_mention_struct) => {
                            // Insert into memcached
                            let key = user_mention_struct.user_name.clone();
                            let value = user_mention_struct.user_id.to_string();
                            mc_client
                                .set(&key, &value, Some(0), None)
                                .await
                                .expect("Failed to set in memcached");

                            let username = if cfg!(debug_assertions) {
                                format!("{}@mongodb", user_mention_struct.user_name)
                            } else {
                                user_mention_struct.user_name.clone()
                            };
                            // Add to user mentions
                            user_mentions.push(UserMention {
                                username,
                                user_id: user_mention_struct.user_id,
                            });
                        }
                        Err(e) => {
                            exception = Some(ServiceException {
                                error_code: ErrorCode::Unknown as i32,
                                message: format!("MongoDB error: {}", e),
                            });
                        }
                    }
                }
            } else {
                exception = Some(ServiceException {
                    error_code: ErrorCode::Unknown as i32,
                    message: "Failed to query MongoDB".to_string(),
                });
            }
        }

        // Return the user mentions
        Ok(Response::new(ComposeUserMentionResponse {
            user_mentions,
            exception,
        }))
    }
}

pub async fn create_service() -> UserMentionServiceServer<UserMentionServiceImpl> {
    let service = match UserMentionServiceImpl::new().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to create UserMentionServiceImpl: {:?}", e);
            panic!("Failed to create UserMentionServiceImpl");
        }
    };
    UserMentionServiceServer::new(service)
}
