use chrono::Utc;
use mongodb::Client as MongoClient;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::url_shorten::db::{
    get_expanded_urls, get_shortened_urls, initialize_database, insert_url_mappings,
};
use crate::url_shorten::{
    url_shorten_service_server::{UrlShortenService, UrlShortenServiceServer},
    ComposeUrlsRequest, ComposeUrlsResponse, ErrorCode, GetExtendedUrlsRequest,
    GetExtendedUrlsResponse, ServiceException, Url,
};

pub mod url_shorten {
    tonic::include_proto!("url_shorten");
}

const HOSTNAME: &str = "http://short-url/";
const RANDOM_STR_LENGTH: usize = 10;

#[derive(Debug)]
pub struct UrlShortenServiceImpl {
    mongo_client: Arc<MongoClient>,
    rng: Arc<tokio::sync::Mutex<StdRng>>,
}

impl UrlShortenServiceImpl {
    pub fn new(mongo_client: MongoClient) -> Self {
        // Seed the RNG with the current timestamp
        let timestamp = Utc::now().timestamp_millis() as u64;
        let rng = StdRng::from_entropy();

        Self {
            mongo_client: Arc::new(mongo_client),
            rng: Arc::new(tokio::sync::Mutex::new(rng)),
        }
    }

    // Generate a random string of specified length
    async fn gen_random_str(&self, length: usize) -> String {
        const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

        let mut rng = self.rng.lock().await;
        let random_string: String = (0..length)
            .map(|_| {
                let idx = rng.gen_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect();

        random_string
    }
}

#[tonic::async_trait]
impl UrlShortenService for UrlShortenServiceImpl {
    async fn compose_urls(
        &self,
        request: Request<ComposeUrlsRequest>,
    ) -> Result<Response<ComposeUrlsResponse>, Status> {
        println!("Got a compose_urls request: {:?}", request);

        let req = request.into_inner();
        let mut urls = req.urls;

        if urls.is_empty() {
            return Ok(Response::new(ComposeUrlsResponse {
                urls: vec![],
                exception: None,
            }));
        }

        // Create shortened URLs
        let mut result_urls = Vec::with_capacity(urls.len());

        // Fetch shortened URLs from MongoDB
        match get_shortened_urls(&self.mongo_client, &urls).await {
            Ok(url_map) => {
                urls.retain(|url| {
                    if let Some(shortened) = url_map.get(url) {
                        result_urls.push(Url {
                            shortened_url: shortened.clone(),
                            expanded_url: url.clone(),
                        });
                        false
                    } else {
                        true
                    }
                });
            }
            Err(e) => {
                println!("MongoDB error: {}", e);
                return Ok(Response::new(ComposeUrlsResponse {
                    urls: vec![],
                    exception: Some(ServiceException {
                        error_code: ErrorCode::SeMongodbError as i32,
                        message: format!("Failed to query URLs from MongoDB: {}", e),
                    }),
                }));
            }
        }

        // If no new URLs to shorten, return the existing ones
        if urls.is_empty() {
            return Ok(Response::new(ComposeUrlsResponse {
                urls: result_urls,
                exception: None,
            }));
        }

        let mut new_urls = Vec::with_capacity(urls.len());
        for expanded_url in urls {
            let random_str = self.gen_random_str(RANDOM_STR_LENGTH).await;
            let shortened_url = format!("{}{}", HOSTNAME, random_str);

            new_urls.push(Url {
                shortened_url,
                expanded_url,
            });
        }

        // Store mappings in MongoDB
        match insert_url_mappings(&self.mongo_client, new_urls.clone()).await {
            Ok(_) => {
                println!("Successfully inserted {} URL mappings", new_urls.len());
            }
            Err(e) => {
                println!("MongoDB error: {}", e);
                return Ok(Response::new(ComposeUrlsResponse {
                    urls: vec![],
                    exception: Some(ServiceException {
                        error_code: ErrorCode::SeMongodbError as i32,
                        message: format!("Failed to insert URLs to MongoDB: {}", e),
                    }),
                }));
            }
        }

        result_urls.extend(new_urls);
        Ok(Response::new(ComposeUrlsResponse {
            urls: result_urls,
            exception: None,
        }))
    }

    async fn get_extended_urls(
        &self,
        request: Request<GetExtendedUrlsRequest>,
    ) -> Result<Response<GetExtendedUrlsResponse>, Status> {
        println!("Got a get_extended_urls request: {:?}", request);

        let req = request.into_inner();
        let shortened_urls = req.shortened_urls;

        if shortened_urls.is_empty() {
            return Ok(Response::new(GetExtendedUrlsResponse {
                expanded_urls: vec![],
                exception: None,
            }));
        }

        // Get URL mappings from MongoDB
        let url_map = match get_expanded_urls(&self.mongo_client, &shortened_urls).await {
            Ok(map) => map,
            Err(e) => {
                println!("MongoDB error: {}", e);
                return Ok(Response::new(GetExtendedUrlsResponse {
                    expanded_urls: vec![],
                    exception: Some(ServiceException {
                        error_code: ErrorCode::SeMongodbError as i32,
                        message: format!("Failed to query URLs from MongoDB: {}", e),
                    }),
                }));
            }
        };

        // Check if we found all the requested URLs
        if url_map.len() < shortened_urls.len() {
            let mut missing_urls = Vec::new();
            for url in &shortened_urls {
                if !url_map.contains_key(url) {
                    missing_urls.push(url.clone());
                }
            }

            // Return error if any URLs were not found
            return Ok(Response::new(GetExtendedUrlsResponse {
                expanded_urls: vec![],
                exception: Some(ServiceException {
                    error_code: ErrorCode::SeThriftHandlerError as i32,
                    message: format!("URLs not found: {}", missing_urls.join(", ")),
                }),
            }));
        }

        // Get expanded URLs in same order as requested
        let mut expanded_urls = Vec::with_capacity(shortened_urls.len());
        for url in shortened_urls {
            if let Some(expanded) = url_map.get(&url) {
                expanded_urls.push(expanded.clone());
            }
        }

        Ok(Response::new(GetExtendedUrlsResponse {
            expanded_urls,
            exception: None,
        }))
    }
}

pub async fn create_service(
) -> Result<UrlShortenServiceServer<UrlShortenServiceImpl>, Box<dyn std::error::Error>> {
    let mongo_client = initialize_database("mongodb://localhost:27017").await?;

    let service = UrlShortenServiceImpl::new(mongo_client);

    Ok(UrlShortenServiceServer::new(service))
}
