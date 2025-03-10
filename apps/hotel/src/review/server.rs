use futures::StreamExt;
use hotel_tonic::review::{review_server::Review, ReviewComm, ReviewResponse};
use mongodb::{bson::doc, Client as MongoClient, Cursor};
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
}

impl ReviewImpl {
    pub async fn new(
        cache_addr: String,
        cache_conn: u32,
        db_addr: String,
    ) -> Result<Self, Box<dyn Error>> {
        // TODO: db/memc setup
        let memc_client = memcache::Client::with_pool_size(cache_addr, cache_conn)?;
        // db::initialize_database
        let mongo_client = MongoClient::with_uri_str(&db_addr).await?;

        Ok(Self {
            memc_client: Arc::new(memc_client),
            mongo_client: Arc::new(mongo_client),
        })
    }
}

#[tonic::async_trait]
impl Review for ReviewImpl {
    async fn get_reviews(
        &self,
        request: Request<hotel_tonic::review::ReviewRequest>,
    ) -> Result<Response<hotel_tonic::review::ReviewResponse>, Status> {
        let request = request.into_inner();
        let mut reviews = Vec::new();
        // TODO: Fix the following:
        // 1) Consider memcached
        // 2) Move db interaction functionality to
        // db.rs
        // Ignoring memcached details for now

        // Just get from mongodb
        // Change database name as well
        // will likely be implemented in db.rs
        let collection = self
            .mongo_client
            .database("review-db")
            .collection::<ReviewComm>("reviews");
        let mut cursor: Cursor<ReviewComm> = collection
            .find(doc! {"hotelId": &(request.hotel_id)}, None)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        while cursor
            .advance()
            .await
            .map_err(|e| Status::internal(e.to_string()))?
        {
            let review = cursor
                .deserialize_current()
                .map_err(|e| Status::internal(e.to_string()))?;
            reviews.push(review);
        }

        Ok(Response::new(ReviewResponse { reviews }))
    }
}
