pub mod hotel {
    pub mod rate {
        tonic::include_proto!("rate");
    }
}
use futures::StreamExt;
use reboot_hotel::FanoutTracker;
use tokio::sync::Mutex;

use std::{collections::HashSet, error::Error, sync::Arc};

use mongodb::{bson::doc, Client as MongoClient};
use tonic::{Request, Response, Status};
use tonic_masa::LatencyTracker;

use crate::db;
use hotel::{rate, rate::rate_server::Rate};

#[allow(unused)]
struct MasaConfig {
    hotels: u32,
    cache_conn: u32,
    cache_miss_rate: u32,
}

pub struct RateImpl {
    memc_client: Arc<memcache::Client>,
    mongo_client: Arc<MongoClient>,
    latency_tracker: Arc<Mutex<LatencyTracker>>,
    fanout_tracker: Arc<FanoutTracker>,
    _config: MasaConfig,
}

impl RateImpl {
    pub async fn new(
        hotels: u32,
        _payload: u32,
        cache_addr: String,
        cache_conn: u32,
        cache_miss_rate: u32,
        db_addr: String,
    ) -> Result<Self, Box<dyn Error>> {
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

        Ok(Self {
            memc_client: Arc::new(memc_client),
            mongo_client: Arc::new(mongo_client),
            latency_tracker,
            fanout_tracker,
            _config: MasaConfig {
                hotels,
                cache_conn,
                cache_miss_rate,
            },
        })
    }
}

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

        // Check memcached first
        let hotel_ids_ref: Vec<_> = request.hotel_ids.iter().map(|id| id.as_str()).collect();
        let memc_resp = self
            .memc_client
            .gets(&hotel_ids_ref)
            .map_err(|e| tonic::Status::internal(format!("Memcached error: {}", e)))?;

        for (hotel_id, item) in memc_resp {
            if let Ok(value) = String::from_utf8(item) {
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

        let rate_plans = Arc::new(Mutex::new(rate_plans));

        // Handle cache misses
        let missing_ids: Vec<String> = rate_set.into_iter().collect();
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
