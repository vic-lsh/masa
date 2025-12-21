use crate::{
    config::{GlobalConfig, ReviewConfig},
    db,
};
use futures::{lock::Mutex, StreamExt};
use hotel_tonic::review::{review_server::Review, ReviewRequest, ReviewResponse};
use masa::LatencyDistribution;
use mongodb::{bson::doc, Client as MongoClient};
use redis::{aio::ConnectionManager as RedisConnectionManager, AsyncCommands};
use std::{error::Error, sync::Arc};
use tonic::{Request, Response, Status};
pub mod hotel_tonic {
    pub mod review {
        tonic::include_proto!("review");
    }
}

const CACHE_TTL_SECS: usize = 300;

pub struct ReviewImpl {
    redis_conn: RedisConnectionManager,
    mongo_client: Arc<MongoClient>,
    latency_tracker: Arc<Mutex<LatencyDistribution>>,
}

impl ReviewImpl {
    pub async fn new(config: ReviewConfig, global: GlobalConfig) -> Result<Self, Box<dyn Error>> {
        let redis_client = redis::Client::open(config.redis_addr.as_str())?;
        let redis_conn = RedisConnectionManager::new(redis_client).await?;
        let mongo_client = db::initialize_database(&config.mongodb_addr).await?;
        let _ = global; // cache_conns is unused while we prototype Redis
        let latency_tracker = Arc::new(Mutex::new(LatencyDistribution::new(
            "ReviewSvc".into(),
            1024,
        )));

        Ok(Self {
            redis_conn,
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

        let hotel_id_ref = hotel_id.as_str();
        // First check redis cache
        let mut redis_conn = self.redis_conn.clone();
        let cached = match redis_conn.get::<_, Option<Vec<u8>>>(hotel_id_ref).await {
            Ok(opt) => opt,
            Err(e) => {
                log::error!("redis get failed: {}", e);
                None
            }
        };

        if let Some(item) = cached {
            log::trace!("redis hit with {}", String::from_utf8_lossy(&item));

            match serde_json::from_slice::<Vec<db::Review>>(&item) {
                Ok(cached_reviews) => {
                    reviews = cached_reviews.into_iter().map(|r| r.into()).collect();
                }
                Err(e) => {
                    log::error!("Failed to unmarshal reviews: {}", e);
                    return Err(Status::internal("Failed to process cached data"));
                }
            }
        } else {
            log::trace!("redis miss, hotelId = {}", hotel_id);

            let collection = self
                .mongo_client
                .database("review-db")
                .collection::<db::Review>("reviews");

            let mut cursor = collection
                .find(doc! {"hotelId": &hotel_id}, None)
                .await
                .map_err(|e| Status::internal(format!("MongoDB error: {}", e)))?;

            let mut db_reviews = Vec::new();

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

            if !db_reviews.is_empty() {
                match serde_json::to_vec(&db_reviews) {
                    Ok(json_bytes) => {
                        let hotel_id_clone = hotel_id.clone();
                        let mut redis_conn = self.redis_conn.clone();
                        tokio::spawn(async move {
                            let res: redis::RedisResult<()> =
                                redis_conn.set_ex(&hotel_id_clone, json_bytes, CACHE_TTL_SECS as u64).await;

                            if let Err(e) = res {
                                log::error!("Failed to set redis cache: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to serialize reviews for caching: {}", e);
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

#[cfg(test)]
mod tests {
    use super::*;
    use redis::AsyncCommands;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    async fn test_redis_conn() -> Option<RedisConnectionManager> {
        let url = std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        let client = redis::Client::open(url).ok()?;
        match RedisConnectionManager::new(client).await {
            Ok(conn) => Some(conn),
            Err(err) => {
                eprintln!("Skipping redis tests; connection failed: {}", err);
                None
            }
        }
    }

    fn sample_reviews(hotel_id: &str) -> Vec<db::Review> {
        serde_json::from_value(json!([
            {
                "reviewId": "redis-1",
                "hotelId": hotel_id,
                "name": "Cache Tester",
                "rating": 4.25,
                "description": "payload should round trip through redis",
                "images": [
                    { "url": "https://example.com/cache.jpg", "default": true }
                ]
            },
            {
                "reviewId": "redis-2",
                "hotelId": hotel_id,
                "name": "Cache Tester 2",
                "rating": 3.8,
                "description": "secondary review for cache test",
                "images": []
            }
        ]))
        .expect("construct sample reviews")
    }

    fn unique_hotel_id(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        format!("{}-{}", prefix, nanos)
    }

    async fn clear_key(conn: &mut RedisConnectionManager, key: &str) {
        let _: redis::RedisResult<()> = redis::cmd("DEL").arg(key).query_async(conn).await;
    }

    #[tokio::test]
    async fn redis_round_trip_stores_and_reads_reviews() {
        let mut conn = match test_redis_conn().await {
            Some(c) => c,
            None => return,
        };

        let hotel_id = unique_hotel_id("redis-round-trip");
        clear_key(&mut conn, &hotel_id).await;

        let reviews = sample_reviews(&hotel_id);
        let payload = serde_json::to_vec(&reviews).expect("serialize reviews");
        conn.set_ex::<&String, Vec<u8>, ()>(&hotel_id, payload, CACHE_TTL_SECS as u64)
            .await
            .expect("redis set_ex failed");

        let cached = conn
            .get::<_, Option<Vec<u8>>>(&hotel_id)
            .await
            .expect("redis get failed")
            .expect("expected redis payload after set_ex");
        let decoded: Vec<db::Review> =
            serde_json::from_slice(&cached).expect("deserialize cached reviews");
        assert_eq!(decoded, reviews);

        let ttl: i64 = conn.ttl(&hotel_id).await.expect("fetch ttl");
        assert!(ttl > 0 && ttl <= CACHE_TTL_SECS as i64);
    }

    #[tokio::test]
    async fn redis_cache_miss_returns_none() {
        let mut conn = match test_redis_conn().await {
            Some(c) => c,
            None => return,
        };

        let hotel_id = unique_hotel_id("redis-miss");
        clear_key(&mut conn, &hotel_id).await;

        let cached: Option<Vec<u8>> = conn.get(&hotel_id).await.expect("redis get failed");
        assert!(cached.is_none());
    }
}
