use async_memcached::AsciiProtocol;
use async_memcached::Client as McClient;
use mongodb::{bson::doc, options::ClientOptions, Client as MongoClient, Collection};
use serde::{Deserialize, Serialize};
use std::env;
use std::error::Error;
use tracing::{error, info};

#[derive(Debug, Serialize, Deserialize)]
pub struct UserMentionStruct {
    #[serde(rename = "user_id")]
    pub user_id: u64,
    #[serde(rename = "username")]
    pub user_name: String,
}

fn generate_static_data() -> Vec<UserMentionStruct> {
    info!("Generating test data...");

    let num_users = 1_000_000; // Generate 1,000,000 entries
    (1..=num_users)
        .map(|id| UserMentionStruct {
            user_id: id as u64,
            user_name: format!("user{}", id),
        })
        .collect()
}

pub async fn initialize_database(url: &str) -> Result<MongoClient, Box<dyn Error>> {
    let new_user_mentions = generate_static_data();

    info!("Attempting connection to {}", url);

    let client_options = ClientOptions::parse(url).await?;
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

pub async fn initialize_memcached(url: &str) -> Result<McClient, Box<dyn Error>> {
    let mut client = McClient::new(url).await?;
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