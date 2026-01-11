pub mod hotel_tonic {
    pub mod profile {
        tonic::include_proto!("profile");
    }
}

#[cfg(not(feature = "synthetic"))]
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
#[cfg(feature = "synthetic")]
use {rand::rngs::StdRng, rand::SeedableRng, rand_distr::Uniform};

use crate::{
    config::{GlobalConfig, ProfileConfig},
    db,
};
use app_utils::stats::latency::{new_latency_tracker, spawn_latency_logger, SyncLatencyTracker};
use mongodb::{bson::doc, Client as MongoClient};
use redis::{aio::ConnectionManager as RedisConnectionManager, AsyncCommands};
#[cfg(feature = "synthetic")]
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

#[cfg(feature = "workload_stats")]
use app_utils::AvgTracker;
use hotel_tonic::{profile, profile::profile_server::Profile};

const CACHE_TTL_SECS: usize = 300;

#[cfg(feature = "synthetic")]
#[allow(unused)]
struct MasaConfig {
    hotels: u32,
    cache_conn: u32,
    cache_miss_rate: u32,
}

#[cfg(feature = "synthetic")]
struct SyntheticProfile {
    rng: Arc<Mutex<StdRng>>,
    uniform: Uniform<u64>,
    config: MasaConfig,
}

pub struct ProfileImpl {
    redis_conn: RedisConnectionManager,
    mongo_client: Arc<MongoClient>,
    latency_tracker: SyncLatencyTracker,
    #[cfg(feature = "workload_stats")]
    fanout_tracker: Arc<AvgTracker>,
    #[cfg(feature = "synthetic")]
    synth: SyntheticProfile,
}

impl ProfileImpl {
    pub async fn new(
        config: ProfileConfig,
        global: GlobalConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        #[cfg(not(feature = "synthetic"))]
        let _ = &global;
        let mongo_client = db::initialize_database(&config.mongodb_addr).await?;

        let (latency_tracker, latency_consumer) = new_latency_tracker("ProfileSvc");
        spawn_latency_logger(latency_consumer, Duration::from_secs(30));

        #[cfg(feature = "workload_stats")]
        let fanout_tracker = {
            let fanout_tracker = Arc::new(AvgTracker::default());
            let fanout = fanout_tracker.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    log::warn!("Avg fanout {}", fanout.get_average_fanout());
                }
            });
            fanout_tracker
        };

        #[cfg(feature = "synthetic")]
        let (rng, uniform) = {
            let seed = 998244353;
            let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(seed)));
            let uniform = Uniform::new(0, 100);
            (rng, uniform)
        };

        #[cfg(feature = "synthetic")]
        let hotels = global.hotels;

        #[cfg(feature = "synthetic")]
        let cache_conn = global.cache_conns;

        #[cfg(feature = "synthetic")]
        let cache_miss_rate = global.prob_cache_miss;

        let redis_client = redis::Client::open(config.redis_addr.as_str())?;
        let redis_conn = RedisConnectionManager::new(redis_client).await?;
        Ok(Self {
            redis_conn,
            mongo_client: Arc::new(mongo_client),
            latency_tracker,
            #[cfg(feature = "workload_stats")]
            fanout_tracker,
            #[cfg(feature = "synthetic")]
            synth: SyntheticProfile {
                rng,
                uniform,
                config: MasaConfig {
                    hotels,
                    cache_conn,
                    cache_miss_rate,
                },
            },
        })
    }
}

#[cfg(not(feature = "synthetic"))]
#[tonic::async_trait]
impl Profile for ProfileImpl {
    async fn get_profiles(
        &self,
        request: Request<profile::ProfileRequest>,
    ) -> Result<Response<profile::ProfileResponse>, Status> {
        let start = std::time::Instant::now();

        let request = request.into_inner();
        // self.fanout_tracker.track(request.hotel_ids.len());

        // Track which hotels need to be fetched from MongoDB
        let mut profile_map: HashSet<String> = request.hotel_ids.iter().cloned().collect();

        let mut hotels = Vec::new();

        // Check redis first
        let hotel_ids_ref: Vec<&str> = request.hotel_ids.iter().map(|id| id.as_str()).collect();
        let mut redis_conn = self.redis_conn.clone();
        let cached_resp: Vec<Option<Vec<u8>>> = redis::cmd("MGET")
            .arg(&hotel_ids_ref)
            .query_async(&mut redis_conn)
            .await
            .unwrap_or_else(|e| {
                log::error!("redis mget failed: {}", e);
                vec![None; hotel_ids_ref.len()]
            });

        for (hotel_id, maybe_bytes) in request.hotel_ids.iter().zip(cached_resp) {
            if let Some(raw) = maybe_bytes {
                if let Ok(value) = String::from_utf8(raw) {
                    if let Ok(hotel) = serde_json::from_str::<db::Hotel>(&value) {
                        hotels.push(hotel);
                        profile_map.remove(hotel_id);
                    }
                }
            }
        }

        // Handle cache misses with MongoDB
        let missing_ids: Vec<String> = profile_map.iter().cloned().collect();

        let mut handles = Vec::new();

        for hotel_id in missing_ids {
            let mongo_client = Arc::clone(&self.mongo_client);
            let redis_conn = self.redis_conn.clone();

            // Spawn a task for each missing hotel
            let handle = tokio::spawn(async move {
                let collection = mongo_client
                    .database("profile-db")
                    .collection::<db::Hotel>("hotels");

                let mut hotels = Vec::new();
                // Query MongoDB
                if let Ok(hotel) = collection.find_one(doc! { "id": &hotel_id }, None).await {
                    if let Some(hotel) = hotel {
                        // Update redis asynchronously
                        if let Ok(prof_json) = serde_json::to_string(&hotel) {
                            let mut redis_conn = redis_conn.clone();
                            if let Err(e) = redis_conn
                                .set_ex::<&std::string::String, std::string::String, ()>(
                                    &hotel_id,
                                    prof_json,
                                    CACHE_TTL_SECS as u64,
                                )
                                .await
                            {
                                log::error!("Failed to set redis cache: {}", e);
                            }
                        }
                        // Update shared hotels vector
                        hotels.push(hotel);
                    }
                }
                hotels
            });

            handles.push(handle);
        }

        // Wait for all MongoDB queries to complete
        for h in handles {
            let new_hotels = h.await.unwrap();
            hotels.extend(new_hotels);
        }

        let hotels = hotels.into_iter().map(|h| h.into()).collect();

        // Create response
        let response = profile::ProfileResponse { hotels };

        let elapsed = start.elapsed().as_micros() as u64;
        self.latency_tracker.track(elapsed);

        Ok(tonic::Response::new(response))
    }
}

#[cfg(feature = "synthetic")]
#[tonic::async_trait]
impl Profile for ProfileImpl {
    async fn get_profiles(
        &self,
        request: Request<profile::ProfileRequest>,
    ) -> Result<Response<profile::ProfileResponse>, Status> {
        let start = std::time::Instant::now();
        let request = request.into_inner();
        let hotels = self.fetch_mixture(request.hotel_ids).await;
        let hotels = hotels.into_iter().map(|h| h.into()).collect();
        let response = profile::ProfileResponse { hotels };
        log::info!("response: {:?}", response);
        self.latency_tracker
            .track(start.elapsed().as_micros().try_into().unwrap());
        Ok(Response::new(response))
    }
}

#[cfg(feature = "synthetic")]
impl ProfileImpl {
    pub async fn fetch_mixture(&self, names: Vec<String>) -> Vec<db::Hotel> {
        use futures::StreamExt;
        use rand_distr::Distribution;

        let names_db = {
            let value = {
                let mut rng = self.synth.rng.lock().await;
                self.synth.uniform.sample(&mut *rng) % 100
            };
            if value < self.synth.config.cache_miss_rate as u64 {
                names.clone()
            } else {
                Vec::new()
            }
        };

        let names_ref = names.iter().map(|s| s.as_str()).collect::<Vec<&str>>();
        let mut hotels = Vec::new();
        let mut redis_conn = self.redis_conn.clone();
        let cached_resp: Vec<Option<String>> = redis::cmd("MGET")
            .arg(&names_ref)
            .query_async(&mut redis_conn)
            .await
            .unwrap_or_else(|e| {
                log::error!("redis mget failed: {}", e);
                vec![None; names_ref.len()]
            });
        for maybe_json in cached_resp {
            if let Some(hotel_json) = maybe_json {
                let hotel = serde_json::from_str(&hotel_json).expect("Failed to deserialize hotel");
                hotels.push(hotel);
            }
        }
        // hotels.sort_by_key(|hotel| hotel.ave);

        if !names_db.is_empty() {
            let query = doc! {
                "name": {
                    "$in": names_db
                }
            };
            let collection = self
                .mongo_client
                .database("profile-db")
                .collection::<db::Hotel>("hotels");
            let mut cursor = collection
                .find(query, None)
                .await
                .expect("Failed to find hotels");
            while let Some(hotel) = cursor.next().await {
                let hotel = hotel.expect("Failed to get hotel");
                let hotel_json = serde_json::to_string(&hotel).expect("Failed to serialize hotel");
                let mut redis_conn = self.redis_conn.clone();
                if let Err(e) = redis_conn
                    .set_ex(hotel.name.as_str(), hotel_json, CACHE_TTL_SECS as u64)
                    .await
                {
                    log::error!("Failed to set redis cache: {}", e);
                }
            }
        }

        hotels
    }
}
