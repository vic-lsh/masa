use async_memcached::AsciiProtocol;
use async_memcached::Client as McClient;
use futures::StreamExt;
use mongodb::bson::{doc, Bson};
use mongodb::Client as MongoClient;
use std::collections::HashSet;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

use user_mention_service::{
    user_mention_service_server::{UserMentionService, UserMentionServiceServer},
    ComposeUserMentionRequest, ComposeUserMentionResponse, ErrorCode, ServiceException,
    UserMention,
};

pub mod user_mention_service {
    tonic::include_proto!("usermention");
}

pub struct UserMentionServiceImpl {
    mc_client: Arc<Mutex<McClient>>,
    mongo_client: Arc<MongoClient>,
}

use crate::user_mention::db::{initialize_database, initialize_memcached, UserMentionStruct};

impl UserMentionServiceImpl {
    pub async fn new() -> Result<Self, Box<dyn Error>> {
        let mongo_client = match initialize_database().await {
            Ok(client) => client,
            Err(e) => {
                eprintln!("Failed to initialize MongoDB: {:?}", e);
                return Err(e);
            }
        };

        let mc_client = match initialize_memcached().await {
            Ok(client) => client,
            Err(e) => {
                eprintln!("Failed to initialize Memcached: {:?}", e);
                return Err(e);
            }
        };

        Ok(Self {
            mc_client: Arc::new(Mutex::new(mc_client)),
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
        println!("Got a request: {:?}", request);

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
        let mc_resp = {
            let mut mc_client = self.mc_client.lock().await;
            mc_client.get_multi(req.usernames.clone()).await
        };

        match mc_resp {
            Err(e) => {
                eprintln!("Memcached Unavailable, falling back to Mongodb: {:?}", e);
            }
            Ok(entries) => {
                for entry in entries {
                    if let Ok(usr_id) = String::from_utf8(entry.data.expect("data must exist")) {
                        let username = String::from_utf8(entry.key).unwrap_or_default();
                        missing_keys.remove(&username);
                        user_mentions.push(UserMention {
                            username: username + "@memcached",
                            user_id: usr_id.parse().unwrap_or_default(),
                        });
                    }
                }
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
                            {
                                let mut mc_client = self.mc_client.lock().await;
                                mc_client
                                    .set(&key, &value, Some(0), None)
                                    .await
                                    .expect("Failed to set in memcached");
                            }

                            // Add to user mentions
                            user_mentions.push(UserMention {
                                username: user_mention_struct.user_name + "@mongodb",
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
