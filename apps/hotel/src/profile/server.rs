pub mod hotel_tonic {
    pub mod profile {
        tonic::include_proto!("profile");
    }
}

#[cfg(not(feature = "synthetic"))]
use std::collections::HashSet;
use std::sync::Arc;
#[cfg(feature = "synthetic")]
use {rand::rngs::StdRng, rand::SeedableRng, rand_distr::Uniform};

use crate::db;
use mongodb::{bson::doc, Client as MongoClient};
use hotel::McPool;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use tonic_masa::LatencyTracker;

use hotel_tonic::{profile, profile::profile_server::Profile};
#[cfg(feature = "workload_stats")]
use hotel::AvgTracker;

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
    mc_pool: Arc<McPool>,
    memc_client: Arc<memcache::Client>,
    mongo_client: Arc<MongoClient>,
    latency_tracker: Arc<Mutex<LatencyTracker>>,
    #[cfg(feature = "workload_stats")]
    fanout_tracker: Arc<AvgTracker>,
    #[cfg(feature = "synthetic")]
    synth: SyntheticProfile,
}

impl ProfileImpl {
    pub async fn new(
        #[allow(unused)] hotels: u32,
        _payload: u32,
        cache_addr: String,
        cache_conn: u32,
        #[allow(unused)] cache_miss_rate: u32,
        db_addr: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let memc_client = memcache::Client::with_pool_size(cache_addr.clone(), cache_conn)?;
        let mongo_client = db::initialize_database(&db_addr).await?;

        let latency_tracker = Arc::new(Mutex::new(LatencyTracker::new("ProfileSvc".into(), 1024)));

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

        let cache_addr = cache_addr
            .strip_prefix("memcache://")
            .map(|addr| format!("tcp://{}", addr))
            .unwrap()
            .to_owned();
        Ok(Self {
            mc_pool: Arc::new(McPool::new(cache_addr, 128)),
            memc_client: Arc::new(memc_client),
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

        let mut mc = self.mc_pool.get().await;
        // let mut mc = async_memcached::Client::new(&self.mc_pool.addr)
        //     .await
        //     .unwrap();
        // Check memcached first
        if let Ok(memc_resp) = mc.get_multi(&request.hotel_ids).await {
            for entry in memc_resp {
                let hotel_id = String::from_utf8(entry.key).unwrap();
                if let Ok(value) = String::from_utf8(entry.data) {
                    if let Ok(hotel) = serde_json::from_str::<db::Hotel>(&value) {
                        hotels.push(hotel);
                        profile_map.remove(&hotel_id);
                    }
                }
            }
        }

        // Handle cache misses with MongoDB
        let missing_ids: Vec<String> = profile_map.iter().cloned().collect();

        let hotels = Arc::new(Mutex::new(hotels));

        let mut handles = Vec::new();

        for hotel_id in missing_ids {
            let hotels = Arc::clone(&hotels);
            let mc_pool = self.mc_pool.clone();
            // let mc_addr = self.mc_pool.addr.clone();
            let mongo_client = Arc::clone(&self.mongo_client);

            // Spawn a task for each missing hotel
            let handle = async_executor::spawn(async move {
                let collection = mongo_client
                    .database("profile-db")
                    .collection::<db::Hotel>("hotels");

                // Query MongoDB
                if let Ok(hotel) = collection.find_one(doc! { "id": &hotel_id }, None).await {
                    if let Some(hotel) = hotel {
                        // Update memcached asynchronously
                        if let Ok(prof_json) = serde_json::to_string(&hotel) {
                            async_executor::spawn(async move {
                                let mut mc = mc_pool.get().await;
                                // let mut mc = async_memcached::Client::new(&mc_addr).await.unwrap();
                                let _ = mc
                                    .set(&hotel_id, prof_json.as_bytes(), None, None)
                                    .await
                                    .ok();
                            })
                            .detach();
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
            h.await;
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

#[cfg(feature = "synthetic")]
#[tonic::async_trait]
impl Profile for ProfileImpl {
    async fn get_profiles(
        &self,
        request: Request<profile::ProfileRequest>,
    ) -> Result<Response<profile::ProfileResponse>, Status> {
        let request = request.into_inner();
        let hotels = self.fetch_mixture(request.hotel_ids).await;
        let hotels = hotels.into_iter().map(|h| h.into()).collect();
        let response = profile::ProfileResponse { hotels };
        log::info!("response: {:?}", response);
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
