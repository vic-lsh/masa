pub mod hotel {
    pub mod profile {
        tonic::include_proto!("profile");
    }
}

use rand::rngs::StdRng;
use rand::SeedableRng;
use rand_distr::Uniform;
use std::{collections::HashSet, sync::Arc};

use crate::db;
use mongodb::{bson::doc, Client as MongoClient};
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use tonic_masa::LatencyTracker;

use hotel::{profile, profile::profile_server::Profile};
use reboot_hotel::FanoutTracker;

#[allow(unused)]
struct MasaConfig {
    hotels: u32,
    cache_conn: u32,
    cache_miss_rate: u32,
}

struct SyntheticProfile {
    rng: Arc<Mutex<StdRng>>,
    uniform: Uniform<u64>,
    config: MasaConfig,
}

pub struct ProfileImpl {
    memc_client: Arc<memcache::Client>,
    mongo_client: Arc<MongoClient>,
    latency_tracker: Arc<Mutex<LatencyTracker>>,
    fanout_tracker: Arc<FanoutTracker>,
    synth: SyntheticProfile,
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

        let latency_tracker = Arc::new(Mutex::new(LatencyTracker::new("ProfileSvc".into(), 1024)));

        let fanout_tracker = Arc::new(FanoutTracker::default());
        // let fanout = fanout_tracker.clone();
        // tokio::spawn(async move {
        //     loop {
        //         tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        //         log::warn!("Avg fanout {}", fanout.get_average_fanout());
        //     }
        // });
        //
        let seed = 998244353;
        let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(seed)));
        let uniform = Uniform::new(0, 100);

        Ok(Self {
            memc_client: Arc::new(memc_client),
            mongo_client: Arc::new(mongo_client),
            latency_tracker,
            fanout_tracker,
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

        let elapsed = start.elapsed().as_micros() as u64;
        {
            self.latency_tracker.lock().await.track(elapsed);
        }

        Ok(tonic::Response::new(response))
    }
}

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
        if let Ok(hotel_jsons) = self.memc_client.gets::<String>(&names_ref) {
            for hotel_json in hotel_jsons.values() {
                let hotel = serde_json::from_str(hotel_json).expect("Failed to deserialize hotel");
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
                self.memc_client
                    .set(hotel.name.as_str(), hotel_json, 0)
                    .expect("Failed to set hotel");
            }
        }

        hotels
    }
}
