use async_memcached::AsciiProtocol;
use async_memcached::Client as McClient;
use mongodb::{bson::doc, options::ClientOptions, Client as MongoClient, Collection};
use serde::{Deserialize, Serialize};
use std::error::Error;
use tracing::{error, info};
use std::env;

#[derive(Debug, Serialize, Deserialize)]
pub struct UserMentionStruct {
    #[serde(rename = "user_id")]
    pub user_id: u64,
    #[serde(rename = "username")]
    pub user_name: String,
}

fn generate_static_data() -> Vec<UserMentionStruct> {
    info!("Generating test data...");

    let new_user_mentions = vec![
        UserMentionStruct {
            user_id: 1,
            user_name: "adam".to_string(),
        },
        UserMentionStruct {
            user_id: 2,
            user_name: "james".to_string(),
        },
        UserMentionStruct {
            user_id: 3,
            user_name: "john".to_string(),
        },
        UserMentionStruct {
            user_id: 4,
            user_name: "alice".to_string(),
        },
        UserMentionStruct {
            user_id: 5,
            user_name: "bob".to_string(),
        },
        UserMentionStruct {
            user_id: 6,
            user_name: "charlie".to_string(),
        },
    ];

    new_user_mentions
}

pub async fn initialize_database() -> Result<MongoClient, Box<dyn Error>> {
    // let new_user_mentions = generate_static_data();

    // let url = "mongodb://127.0.0.1:27017";
    // info!("Attempting connection to {}", url);

    // let client_options = ClientOptions::parse(&url).await?;
    // let client = MongoClient::with_options(client_options)?;
    // info!("Successfully connected to MongoDB");

    let new_user_mentions = generate_static_data();

    // Read the URL from the environment variable
    let url = env::var("MONGO_URL")
        .expect("MONGO_URL environment variable must be set");

    info!("Attempting connection to {}", url);

    let client_options = ClientOptions::parse(&url).await?;
    let client = MongoClient::with_options(client_options)?;
    info!("Successfully connected to MongoDB");

    let database = client.database("usermention-db");
    let usermention_collection: Collection<UserMentionStruct> = database.collection("usermention");

    usermention_collection
        .delete_many(doc! {}, None)
        .await
        .map_err(|e| {
            error!("Failed to clean usermention collection: {}", e);
            e
        })?;

    // Insert reservations
    usermention_collection
        .insert_many(new_user_mentions, None)
        .await
        .map_err(|e| {
            error!("Failed to insert usermention: {}", e);
            e
        })?;

    info!("Successfully inserted test data into usermention DB");

    Ok(client)
}

pub async fn initialize_memcached() -> Result<McClient, Box<dyn Error>> {
    // let url = "tcp://127.0.0.1:11211";
    // let mut client = McClient::new(url).await?;
    // info!("Successfully connected to Memcached");

    // Read the URL from the environment variable
    let url = env::var("MEMCACHED_URL")
        .expect("MEMCACHED_URL environment variable must be set");

    // let mut client = McClient::new(&url).await?; // Pass the URL as a reference
    // let mut client: McClient<AsciiProtocol<tokio::net::TcpStream>> = McClient::new(&url).await?;
    let mut client = McClient::new(&url).await?;
    info!("Successfully connected to Memcached");

    // Clean the memcached
    client.flush_all().await?;

    let keys = vec!["adam", "james", "john"];

    let values = vec!["1", "2", "3"];

    for (key, value) in keys.iter().zip(values.iter()) {
        client.set(*key, *value, Some(0), None).await?;
    }

    Ok(client)
}
