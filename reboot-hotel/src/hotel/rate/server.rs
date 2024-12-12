pub mod hotel {
    pub mod rate {
        tonic::include_proto!("rate");
    }
}
use futures::StreamExt;
#[cfg(feature = "workload_stats")]
use reboot_hotel::AvgTracker;
use reboot_hotel::McPool;
#[cfg(not(feature = "synthetic"))]
use std::collections::HashSet;
use tokio::sync::Mutex;
#[cfg(feature = "synthetic")]
use {
    rand::rngs::StdRng,
    rand::SeedableRng,
    rand_distr::{Distribution, Uniform},
};

use std::{error::Error, sync::Arc};

use mongodb::{bson::doc, Client as MongoClient};
use tonic::{Request, Response, Status};
use tonic_masa::LatencyTracker;

use crate::db;
use hotel::{rate, rate::rate_server::Rate};

#[cfg(feature = "synthetic")]
#[allow(unused)]
struct MasaConfig {
    hotels: u32,
    cache_conn: u32,
    cache_miss_rate: u32,
}

#[cfg(feature = "synthetic")]
struct SyntheticRate {
    rng: Arc<Mutex<StdRng>>,
    uniform: Uniform<u64>,
    config: MasaConfig,
}

pub struct RateImpl {
    mc_pool: Arc<McPool>,
    // memc_client: Arc<memcache::Client>,
    mongo_client: Arc<MongoClient>,
    latency_tracker: Arc<Mutex<LatencyTracker>>,
    #[cfg(feature = "workload_stats")]
    fanout_tracker: Arc<AvgTracker>,
    #[cfg(feature = "synthetic")]
    synth: SyntheticRate,
}

impl RateImpl {
    pub async fn new(
        #[allow(unused)] hotels: u32,
        cache_addr: String,
        cache_conn: u32,
        #[allow(unused)] cache_miss_rate: u32,
        db_addr: String,
    ) -> Result<Self, Box<dyn Error>> {
        // let memc_client = memcache::Client::with_pool_size(cache_addr, cache_conn)?;
        let mongo_client = db::initialize_database(&db_addr).await?;

        let latency_tracker = Arc::new(Mutex::new(LatencyTracker::new("RateSvc".into(), 1024)));

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
            mc_pool: Arc::new(McPool::new(cache_addr, 256)),
            // memc_client: Arc::new(memc_client),
            mongo_client: Arc::new(mongo_client),
            latency_tracker,
            #[cfg(feature = "workload_stats")]
            fanout_tracker,
            #[cfg(feature = "synthetic")]
            synth: SyntheticRate {
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
impl Rate for RateImpl {
    async fn get_rates(
        &self,
        request: Request<rate::RateRequest>,
    ) -> Result<Response<rate::RateResponse>, Status> {
        let start = std::time::Instant::now();

        let request = request.into_inner();

        // Create a set of hotel IDs for tracking missing cache entries
        let mut rate_set: HashSet<String> = request.hotel_ids.iter().cloned().collect();

        // self.fanout_tracker.track(request.hotel_ids.len());

        let mut rate_plans = Vec::new();

        let mut mc = self.mc_pool.get().await;
        // Check memcached first
        if let Ok(mc_resp) = mc.get_multi(&request.hotel_ids).await {
            for entry in mc_resp {
                let hotel_id = String::from_utf8(entry.key).expect("hotel id should be valid");
                if let Ok(value) = String::from_utf8(entry.data) {
                    for rate_str in value.split('\n') {
                        if !rate_str.is_empty() {
                            if let Ok(rate_plan) = serde_json::from_str::<db::RatePlan>(rate_str) {
                                rate_plans.push(rate_plan);
                            }
                        }
                    }
                    rate_set.remove(&hotel_id);
                }
            }
        }

        let rate_plans = Arc::new(Mutex::new(rate_plans));

        // Handle cache misses
        let missing_ids: Vec<String> = rate_set.into_iter().collect();
        let handles: Vec<_> = missing_ids
            .into_iter()
            .map(|hotel_id| {
                let rate_plans_clone = Arc::clone(&rate_plans);
                let mongo_client = Arc::clone(&self.mongo_client);
                let mc_pool = Arc::clone(&self.mc_pool);

                tokio::spawn(async move {
                    let collection = mongo_client
                        .database("rate-db")
                        .collection::<db::RatePlan>("inventory");

                    let mut cursor = collection
                        .find(doc! {}, None)
                        .await
                        .expect("failed to find rate");
                    let mut tmp_rate_plans = Vec::new();
                    let mut memc_str = String::new();

                    while let Some(Ok(rate_plan)) = cursor.next().await {
                        if let Ok(rate_json) = serde_json::to_string(&rate_plan) {
                            memc_str.push_str(&rate_json);
                            memc_str.push('\n');
                        }
                        tmp_rate_plans.push(rate_plan);
                    }

                    // Update rate plans
                    {
                        let mut rate_plans = rate_plans_clone.lock().await;
                        rate_plans.extend(tmp_rate_plans);
                    }

                    // Update memcached asynchronously
                    if !memc_str.is_empty() {
                        tokio::spawn(async move {
                            let mut mc = mc_pool.get().await;
                            let _ = mc.set(&hotel_id, memc_str.as_bytes(), None, None).await;
                        });
                    }
                })
            })
            .collect();

        for h in handles {
            h.await.unwrap();
        }

        // Sort rate plans
        let mut final_rate_plans = Arc::into_inner(rate_plans)
            .expect("rate plan clones should have been dropped")
            .into_inner();
        final_rate_plans.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let response = rate::RateResponse {
            rate_plans: final_rate_plans.into_iter().map(|p| p.into()).collect(),
        };
        log::info!("response: {:?}", response);
        let end = start.elapsed();
        {
            self.latency_tracker
                .lock()
                .await
                .track(end.as_micros().try_into().unwrap());
        }
        Ok(Response::new(response))
    }
}

#[cfg(feature = "synthetic")]
#[tonic::async_trait]
impl Rate for RateImpl {
    async fn get_rates(
        &self,
        request: Request<rate::RateRequest>,
    ) -> Result<Response<rate::RateResponse>, Status> {
        let start = std::time::Instant::now();

        let request = request.into_inner();
        let missing_ids = {
            let mut rng = self.synth.rng.lock().await;
            let value = self.synth.uniform.sample(&mut *rng) % 100;
            if value < self.synth.config.cache_miss_rate as u64 {
                request.hotel_ids.clone()
            } else {
                Vec::new()
            }
        };

        // self.fanout_tracker.track(request.hotel_ids.len());

        let mut rate_plans = Vec::new();

        // Check memcached first
        let hotel_ids_ref: Vec<_> = request.hotel_ids.iter().map(|id| id.as_str()).collect();
        let memc_resp = self
            .memc_client
            .gets(&hotel_ids_ref)
            .map_err(|e| tonic::Status::internal(format!("Memcached error: {}", e)))?;

        for (_hotel_id, item) in memc_resp {
            if let Ok(value) = String::from_utf8(item) {
                for rate_str in value.split('\n') {
                    if !rate_str.is_empty() {
                        if let Ok(rate_plan) = serde_json::from_str::<db::RatePlan>(rate_str) {
                            rate_plans.push(rate_plan);
                        }
                    }
                }
            }
        }

        let rate_plans = Arc::new(Mutex::new(rate_plans));

        // Handle cache misses
        let handles: Vec<_> = missing_ids
            .into_iter()
            .map(|hotel_id| {
                let rate_plans_clone = Arc::clone(&rate_plans);
                let mongo_client = Arc::clone(&self.mongo_client);
                let memc_client = Arc::clone(&self.memc_client);

                tokio::spawn(async move {
                    let collection = mongo_client
                        .database("rate-db")
                        .collection::<db::RatePlan>("inventory");

                    let mut cursor = collection
                        .find(doc! {}, None)
                        .await
                        .expect("failed to find rate");
                    let mut tmp_rate_plans = Vec::new();
                    let mut memc_str = String::new();

                    while let Some(Ok(rate_plan)) = cursor.next().await {
                        if let Ok(rate_json) = serde_json::to_string(&rate_plan) {
                            memc_str.push_str(&rate_json);
                            memc_str.push('\n');
                        }
                        tmp_rate_plans.push(rate_plan);
                    }

                    // Update rate plans
                    {
                        let mut rate_plans = rate_plans_clone.lock().await;
                        rate_plans.extend(tmp_rate_plans);
                    }

                    // Update memcached asynchronously
                    if !memc_str.is_empty() {
                        tokio::spawn(async move {
                            let _ = memc_client.set(&hotel_id, memc_str.as_bytes(), 0);
                        });
                    }
                })
            })
            .collect();

        for h in handles {
            h.await.unwrap();
        }

        // Sort rate plans
        let mut final_rate_plans = Arc::into_inner(rate_plans)
            .expect("rate plan clones should have been dropped")
            .into_inner();
        final_rate_plans.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let response = rate::RateResponse {
            rate_plans: final_rate_plans.into_iter().map(|p| p.into()).collect(),
        };
        log::info!("response: {:?}", response);
        let end = start.elapsed();
        {
            self.latency_tracker
                .lock()
                .await
                .track(end.as_micros().try_into().unwrap());
        }
        Ok(Response::new(response))
    }
}
