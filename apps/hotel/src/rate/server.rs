pub mod hotel_tonic {
    pub mod rate {
        tonic::include_proto!("rate");
    }
}
#[cfg(feature = "workload_stats")]
use app_utils::AvgTracker;
use futures::StreamExt;
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

use masa::LatencyDistribution;
use mongodb::{bson::doc, Client as MongoClient};
use redis::{aio::ConnectionManager as RedisConnectionManager, AsyncCommands};
use tonic::{Request, Response, Status};

use crate::{
    config::{GlobalConfig, RateConfig},
    db,
};
use hotel_tonic::{rate, rate::rate_server::Rate};

const CACHE_TTL_SECS: usize = 300;

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
    redis_conn: RedisConnectionManager,
    mongo_client: Arc<MongoClient>,
    latency_tracker: Arc<Mutex<LatencyDistribution>>,
    #[cfg(feature = "workload_stats")]
    fanout_tracker: Arc<AvgTracker>,
    #[cfg(feature = "synthetic")]
    synth: SyntheticRate,
}

impl RateImpl {
    pub async fn new(config: RateConfig, global: GlobalConfig) -> Result<Self, Box<dyn Error>> {
        #[cfg(not(feature = "synthetic"))]
        let _ = &global;
        let mongo_client = db::initialize_database(&config.mongodb_addr).await?;

        let latency_tracker =
            Arc::new(Mutex::new(LatencyDistribution::new("RateSvc".into(), 1024)));

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
                    for rate_str in value.split('\n') {
                        if !rate_str.is_empty() {
                            if let Ok(rate_plan) = serde_json::from_str::<db::RatePlan>(rate_str) {
                                rate_plans.push(rate_plan);
                            }
                        }
                    }
                    rate_set.remove(hotel_id);
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
                let redis_conn = self.redis_conn.clone();

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

                    // Update redis asynchronously
                    if !memc_str.is_empty() {
                        let mut redis_conn = redis_conn.clone();
                        if let Err(e) = redis_conn
                            .set_ex::<&std::string::String, std::string::String, ()>(
                                &hotel_id,
                                memc_str,
                                CACHE_TTL_SECS as u64,
                            )
                            .await
                        {
                            log::error!("Failed to set redis cache: {}", e);
                        }
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

        // Check redis first
        let hotel_ids_ref: Vec<_> = request.hotel_ids.iter().map(|id| id.as_str()).collect();
        let mut redis_conn = self.redis_conn.clone();
        let cached_resp: Vec<Option<Vec<u8>>> = redis::cmd("MGET")
            .arg(&hotel_ids_ref)
            .query_async(&mut redis_conn)
            .await
            .unwrap_or_else(|e| {
                log::error!("redis mget failed: {}", e);
                vec![None; hotel_ids_ref.len()]
            });

        for maybe_bytes in cached_resp {
            if let Some(raw) = maybe_bytes {
                if let Ok(value) = String::from_utf8(raw) {
                    for rate_str in value.split('\n') {
                        if !rate_str.is_empty() {
                            if let Ok(rate_plan) = serde_json::from_str::<db::RatePlan>(rate_str) {
                                rate_plans.push(rate_plan);
                            }
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
                let redis_conn = self.redis_conn.clone();

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

                    // Update redis asynchronously
                    if !memc_str.is_empty() {
                        let mut redis_conn = redis_conn.clone();
                        if let Err(e) = redis_conn
                            .set_ex(&hotel_id, memc_str, CACHE_TTL_SECS as u64)
                            .await
                        {
                            log::error!("Failed to set redis cache: {}", e);
                        }
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
