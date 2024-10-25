pub mod hotel {
    pub mod profile {
        tonic::include_proto!("profile");
    }
}

use std::{collections::HashSet, sync::Arc};

use crate::db;
use mongodb::{bson::doc, Client as MongoClient};
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

use hotel::{profile, profile::profile_server::Profile};

#[allow(unused)]
struct MasaConfig {
    hotels: u32,
    cache_conn: u32,
    cache_miss_rate: u32,
}

pub struct ProfileImpl {
    memc_client: Arc<memcache::Client>,
    mongo_client: Arc<MongoClient>,
    _config: MasaConfig,
}

impl ProfileImpl {
    pub async fn new(
        hotels: u32,
        _payload: u32,
        cache_addr: String,
        cache_conn: u32,
        cache_miss_rate: u32,
        db_addr: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let memc_client = memcache::Client::with_pool_size(cache_addr, cache_conn)?;
        let mongo_client = db::initialize_database(&db_addr).await?;

        Ok(Self {
            memc_client: Arc::new(memc_client),
            mongo_client: Arc::new(mongo_client),
            _config: MasaConfig {
                hotels,
                cache_conn,
                cache_miss_rate,
            },
        })
    }
}

#[tonic::async_trait]
impl Profile for ProfileImpl {
    async fn get_profiles(
        &self,
        request: Request<profile::ProfileRequest>,
    ) -> Result<Response<profile::ProfileResponse>, Status> {
        let request = request.into_inner();

        // Track which hotels need to be fetched from MongoDB
        let mut profile_map: HashSet<String> = request.hotel_ids.iter().cloned().collect();

        let mut hotels = Vec::new();

        // Check memcached first
        let hotel_ids_ref: Vec<_> = request.hotel_ids.iter().map(|id| id.as_str()).collect();
        let memc_resp = self
            .memc_client
            .gets(&hotel_ids_ref)
            .map_err(|e| tonic::Status::internal(format!("Memcached error: {}", e)))?;
        for (hotel_id, item) in memc_resp {
            if let Ok(value) = String::from_utf8(item) {
                if let Ok(hotel) = serde_json::from_str::<db::Hotel>(&value) {
                    hotels.push(hotel);
                    profile_map.remove(&hotel_id);
                }
            }
        }

        // Handle cache misses with MongoDB
        let missing_ids: Vec<String> = profile_map.iter().cloned().collect();

        let hotels = Arc::new(Mutex::new(hotels));

        let mut handles = Vec::new();

        for hotel_id in missing_ids {
            let hotels = Arc::clone(&hotels);
            let mongo_client = Arc::clone(&self.mongo_client);
            let memc_client = Arc::clone(&self.memc_client);

            // Spawn a task for each missing hotel
            let handle = tokio::spawn(async move {
                let collection = mongo_client
                    .database("profile-db")
                    .collection::<db::Hotel>("hotels");

                // Query MongoDB
                if let Ok(hotel) = collection.find_one(doc! { "id": &hotel_id }, None).await {
                    if let Some(hotel) = hotel {
                        // Update memcached asynchronously
                        if let Ok(prof_json) = serde_json::to_string(&hotel) {
                            tokio::spawn(async move {
                                let _ = memc_client.set(&hotel_id, prof_json.as_bytes(), 0);
                            });
                        }
                        // Update shared hotels vector
                        hotels.lock().await.push(hotel);
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for all MongoDB queries to complete
        for h in handles {
            h.await.unwrap();
        }

        let hotels = Arc::into_inner(hotels)
            .expect("all clones should have been dropped")
            .into_inner()
            .into_iter()
            .map(|h| h.into())
            .collect();

        // Create response
        let response = profile::ProfileResponse { hotels };

        Ok(tonic::Response::new(response))
    }
}
