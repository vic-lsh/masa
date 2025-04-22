use futures::TryStreamExt;
use mongodb::bson::Document;
use mongodb::options::IndexOptions;
use mongodb::IndexModel;
use mongodb::{bson::doc, options::ClientOptions, Client, Collection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::url_shorten::Url;

// Constants
const DB_NAME: &str = "url-shorten";
const COLLECTION_NAME: &str = "url-mappings";

#[derive(Debug, Serialize, Deserialize)]
pub struct UrlMapping {
    pub shortened_url: String,
    pub expanded_url: String,
}

impl From<UrlMapping> for Url {
    fn from(mapping: UrlMapping) -> Self {
        Self {
            shortened_url: mapping.shortened_url,
            expanded_url: mapping.expanded_url,
        }
    }
}

pub async fn initialize_database(url: &str) -> Result<Client, Box<dyn std::error::Error>> {
    println!("Attempting connection to {}", url);

    let client_options = ClientOptions::parse(url).await?;
    let client = Client::with_options(client_options)?;

    client.database(DB_NAME).list_collection_names(None).await?;
    println!("Successfully connected to MongoDB at {}", url);

    // Create index on shortened_url for faster lookups
    let collection = get_collection(&client);
    collection
        .delete_many(doc! {}, None)
        .await
        .map_err(|e| {
            eprintln!("Failed to clean collection {}: {}", COLLECTION_NAME, e);
            e
        })?;

    let options = IndexOptions::builder().unique(true).build();
    let model = IndexModel::builder()
        .keys(doc! { "shortened_url": 1 })
        .options(options)
        .build();
    collection.create_index(model, None).await?;
    println!("Created index on shortened_url field");

    Ok(client)
}

pub fn get_collection(client: &Client) -> Collection<Document> {
    client.database(DB_NAME).collection(COLLECTION_NAME)
}

pub async fn insert_url_mappings(
    client: &Client,
    mappings: Vec<Url>,
) -> Result<(), Box<dyn std::error::Error>> {
    if mappings.is_empty() {
        return Ok(());
    }

    let collection = get_collection(client);

    let documents: Vec<Document> = mappings
        .iter()
        .map(|url| {
            doc! {
                "shortened_url": &url.shortened_url,
                "expanded_url": &url.expanded_url,
            }
        })
        .collect();

    collection.insert_many(documents, None).await?;
    println!("Inserted {} URL mappings into MongoDB", mappings.len());

    Ok(())
}

pub async fn get_expanded_urls(
    client: &Client,
    shortened_urls: &[String],
) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    if shortened_urls.is_empty() {
        return Ok(HashMap::new());
    }

    let collection = get_collection(client);
    let filter = doc! {
        "shortened_url": {
            "$in": shortened_urls
        }
    };

    let mut cursor = collection.find(filter, None).await?;
    let mut url_map = HashMap::new();

    while let Some(result) = cursor.try_next().await? {
        if let (Ok(shortened), Ok(expanded)) = (
            result.get_str("shortened_url"),
            result.get_str("expanded_url"),
        ) {
            url_map.insert(shortened.to_owned(), expanded.to_owned());
        }
    }

    println!("Retrieved {} URL mappings from MongoDB", url_map.len());

    Ok(url_map)
}
