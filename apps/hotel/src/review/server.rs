use crate::{config::HotelConfig, db};
use futures::{lock::Mutex, StreamExt};
use hotel_tonic::review::{review_server::Review, ReviewComm, ReviewRequest, ReviewResponse};
use masa::LatencyTracker;
use mongodb::{bson::doc, Client as MongoClient};
use std::{error::Error, sync::Arc};
use tonic::{Request, Response, Status};
pub mod hotel_tonic {
    pub mod review {
        tonic::include_proto!("review");
    }
}

pub struct ReviewImpl {
    memc_client: Arc<memcache::Client>,
    mongo_client: Arc<MongoClient>,
    latency_tracker: Arc<Mutex<LatencyTracker>>,
}

impl ReviewImpl {
    pub async fn new(config: HotelConfig) -> Result<Self, Box<dyn Error>> {
        let memc_client =
            memcache::Client::with_pool_size(config.review_memcached_addr, config.cache_conns)?;
        let mongo_client = db::initialize_database(&config.review_mongodb_addr).await?;
        let latency_tracker = Arc::new(Mutex::new(LatencyTracker::new("ReviewSvc".into(), 1024)));

        Ok(Self {
            memc_client: Arc::new(memc_client),
            mongo_client: Arc::new(mongo_client),
            latency_tracker,
        })
    }
}

#[tonic::async_trait]
impl Review for ReviewImpl {
    async fn get_reviews(
        &self,
        request: Request<ReviewRequest>,
    ) -> Result<Response<ReviewResponse>, Status> {
        let start = std::time::Instant::now();
        let request = request.into_inner();
        let hotel_id = request.hotel_id;

        let mut reviews = Vec::new();

        // First check memcached
        let hotel_id_ref = hotel_id.as_str();
        match self.memc_client.get::<Vec<u8>>(hotel_id_ref) {
            Ok(Some(item)) => {
                // Memcached hit

                log::trace!("memc hit with {}", String::from_utf8_lossy(&item));

                // Deserialize the JSON string into reviews
                match serde_json::from_slice::<Vec<db::Review>>(&item) {
                    Ok(cached_reviews) => {
                        // Convert db::Review to ReviewComm
                        reviews = cached_reviews.into_iter().map(|r| r.into()).collect();
                    }
                    Err(e) => {
                        log::error!("Failed to unmarshal reviews: {}", e);
                        return Err(Status::internal("Failed to process cached data"));
                    }
                }
            }
            Ok(None) | Err(_) => {
                // Memcached miss or other error, fetch from MongoDB
                log::trace!("memc miss, hotelId = {}", hotel_id);

                let collection = self
                    .mongo_client
                    .database("review-db")
                    .collection::<db::Review>("reviews");

                // Find reviews matching the hotel ID
                let mut cursor = collection
                    .find(doc! {"hotelId": &hotel_id}, None)
                    .await
                    .map_err(|e| Status::internal(format!("MongoDB error: {}", e)))?;

                let mut db_reviews = Vec::new();

                // Iterate through cursor and collect reviews
                while let Some(result) = cursor.next().await {
                    match result {
                        Ok(review) => {
                            db_reviews.push(review.clone());
                            reviews.push(review.into());
                        }
                        Err(e) => {
                            log::error!("Error deserializing review: {}", e);
                            return Err(Status::internal("Failed to process database records"));
                        }
                    }
                }

                // Cache the results in memcache
                if !db_reviews.is_empty() {
                    match serde_json::to_vec(&db_reviews) {
                        Ok(json_bytes) => {
                            // Store in memcached asynchronously
                            let memc_client = Arc::clone(&self.memc_client);
                            let hotel_id_clone = hotel_id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = memc_client.set(&hotel_id_clone, &json_bytes[..], 0)
                                {
                                    log::error!("Failed to set memcached: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            log::error!("Failed to serialize reviews for caching: {}", e);
                        }
                    }
                }
            }
        }

        // Track latency
        let elapsed = start.elapsed();
        {
            let mut tracker = self.latency_tracker.lock().await;
            tracker.track(elapsed.as_micros() as u64);
        }

        Ok(Response::new(ReviewResponse { reviews }))
    }
}
