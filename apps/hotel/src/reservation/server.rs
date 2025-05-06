pub mod hotel_tonic {
    pub mod reservation {
        tonic::include_proto!("reservation");
    }
}
use async_memcached::AsciiProtocol;
use chrono::DateTime;
use hotel::AvgTracker;
use hotel_tonic::reservation::{self, reservation_server::Reservation};
use masa::LatencyDistribution;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::HotelConfig;
use crate::db;
use app_util_macros::track_latency;
use app_utils::latency::{new_latency_tracker, SyncLatencyTracker};
use hotel::McPool;
use mongodb::{bson::doc, Client as MongoClient, Collection};
use tonic::{Request, Response, Status};

// Seems like it's a memcached library-level bug to have protocol error.
// This function detects protocol errors.
// Upon protocol errors, we reset the client connection.
fn is_mc_protocol_err<T>(mc_resp: &Result<T, async_memcached::Error>) -> bool {
    matches!(mc_resp, Err(async_memcached::Error::Protocol(_)))
}

struct CheckAvailStats {
    e2e: SyncLatencyTracker,
    mc_get_capacity: SyncLatencyTracker,
    mongo_get_capacity: SyncLatencyTracker,
    mc_set_capacity: SyncLatencyTracker,
    mc_get_reservations: SyncLatencyTracker,
    mongo_get_reservations: SyncLatencyTracker,
    mc_set_reservations: SyncLatencyTracker,
}

impl CheckAvailStats {
    fn new() -> Self {
        let (e2e, e2e_consumer) = new_latency_tracker("e2e");
        let (mc_get_capacity, mc_get_capacity_consumer) = new_latency_tracker("mc_get_capacity");
        let (mongo_get_capacity, mongo_get_capacity_consumer) =
            new_latency_tracker("mongo_get_capacity");
        let (mc_set_capacity, mc_set_capacity_consumer) = new_latency_tracker("mc_set_capacity");
        let (mc_get_reservations, mc_get_reservations_consumer) =
            new_latency_tracker("mc_get_reservations");
        let (mc_set_reservations, mc_set_reservations_consumer) =
            new_latency_tracker("mc_set_reservations");
        let (mongo_get_reservations, mongo_get_reservations_consumer) =
            new_latency_tracker("mongo_get_reservations");

        tokio::spawn(async move {
            let percentiles = [50.0, 90.0, 99.0, 99.9];
            let mut latency_consumers = [
                e2e_consumer,
                mc_get_capacity_consumer,
                mongo_get_capacity_consumer,
                mc_set_capacity_consumer,
                mc_get_reservations_consumer,
                mongo_get_reservations_consumer,
                mc_set_reservations_consumer,
            ];

            let name_width = latency_consumers
                .iter()
                .map(|c| c.name.len())
                .max()
                .expect("max must exist")
                + 4;

            let mut secs = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                secs += 1;
                println!("#{}", secs);
                for c in latency_consumers.iter_mut() {
                    let mut dist = c.consume();
                    print!("{: <width$}", c.name, width = name_width);
                    print!("# recs {: <width$}", dist.len(), width = 6);
                    for p in percentiles {
                        print!("p{}: {} ", p, dist.percentile(p));
                    }
                    print!("\n");
                }
                print!("\n");
            }
        });

        Self {
            e2e,
            mc_get_capacity,
            mongo_get_capacity,
            mc_set_capacity,
            mc_get_reservations,
            mc_set_reservations,
            mongo_get_reservations,
        }
    }
}

pub struct ReservationImpl {
    mc_pool: Arc<McPool>,
    mongo_client: MongoClient,
    mongo_reserve_client: MongoClient,
    check_avail_mc_hotel_cap: Arc<AvgTracker>,
    check_avail_mongo_hotel_cap: Arc<AvgTracker>,
    check_avail_mc_reserve: Arc<AvgTracker>,
    check_avail_mongo_reserve: Arc<AvgTracker>,
    mk_reserve_mongo: Arc<AvgTracker>,
    mc_err_count: Arc<AtomicUsize>,

    check_avail_stats: Arc<CheckAvailStats>,
}

impl ReservationImpl {
    pub async fn new(config: HotelConfig) -> Result<Self, Box<dyn Error>> {
        let mongo_client = crate::db::initialize_database(&config.reservation_mongodb_addr).await?;
        let client_options =
            mongodb::options::ClientOptions::parse(&config.reservation_mongodb_addr).await?;
        let mongo_reserve_client = MongoClient::with_options(client_options)?;

        let check_avail_mc_hotel_cap = Arc::new(AvgTracker::default());
        let check_avail_mongo_hotel_cap = Arc::new(AvgTracker::default());

        let check_avail_mc_reserve = Arc::new(AvgTracker::default());
        let check_avail_mongo_reserve = Arc::new(AvgTracker::default());

        let mk_reserve_mongo = Arc::new(AvgTracker::default());

        let mc_err_count = Arc::new(AtomicUsize::new(0));

        let cache_addr = config
            .reservation_memcached_addr
            .strip_prefix("memcache://")
            .map(|addr| format!("tcp://{}", addr))
            .unwrap()
            .to_owned();

        Ok(Self {
            mc_pool: Arc::new(McPool::new(cache_addr, 32)),
            mongo_client,
            mongo_reserve_client,
            check_avail_mc_reserve,
            check_avail_mongo_reserve,
            check_avail_mc_hotel_cap,
            check_avail_mongo_hotel_cap,
            mk_reserve_mongo,
            mc_err_count,

            check_avail_stats: Arc::new(CheckAvailStats::new()),
        })
    }
}

#[tonic::async_trait]
impl Reservation for ReservationImpl {
    async fn check_availability(
        &self,
        req: Request<reservation::ReservationRequest>,
    ) -> Result<Response<reservation::ReservationResponse>, Status> {
        use futures::StreamExt;

        // even with this the app still hangs under high load...
        // scheduler bug?
        const MC_TIMEOUT: Duration = Duration::from_millis(100);

        let start = Instant::now();

        let req = req.into_inner();

        // Create hotel memory keys and maps
        let mut hotel_mem_keys = Vec::new();
        let mut missing_keys = HashSet::new();
        let mut res_map: HashMap<String, bool> = HashMap::new();

        self.check_avail_mc_hotel_cap.track(req.hotel_ids.len());
        for hotel_id in &req.hotel_ids {
            let cap_key = format!("{}_cap", hotel_id);
            hotel_mem_keys.push(cap_key.clone());
            missing_keys.insert(cap_key);
            res_map.insert(hotel_id.clone(), true);
        }

        // Get capacity from memcached
        let mut cache_cap = HashMap::new();

        let mut mc_client = self.mc_pool.get().await;

        let mc_resp = track_latency!(self.check_avail_stats.mc_get_capacity, {
            tokio::time::timeout(MC_TIMEOUT, mc_client.get_multi(hotel_mem_keys)).await
        });
        if let Ok(Ok(mc_resp)) = mc_resp {
            for entry in mc_resp {
                if let Ok(cap) = String::from_utf8(entry.data.expect("data must exist"))
                    .unwrap_or_default()
                    .parse::<i32>()
                {
                    let hotel_id = String::from_utf8(entry.key).unwrap();
                    missing_keys.remove(&hotel_id);
                    cache_cap.insert(hotel_id, cap);
                }
            }
        }

        // let max_missing_keys = 2;
        // let mut missing_keys: HashSet<_> = missing_keys.drain().take(max_missing_keys).collect();
        self.check_avail_mongo_hotel_cap.track(missing_keys.len());
        // Handle cache misses with MongoDB
        if !missing_keys.is_empty() {
            let num_collection = self
                .mongo_client
                .database("reservation-db")
                .collection::<db::Number>("number");

            let query_miss_keys: Vec<String> = missing_keys
                .iter()
                .map(|k| k.split('_').next().unwrap().to_string())
                .collect();

            let cursor = num_collection
                .find(doc! { "hotelId": { "$in": &query_miss_keys } }, None)
                .await
                .map_err(|e| tonic::Status::internal(format!("mongo error: {}", e)))?;

            let results = track_latency!(self.check_avail_stats.mongo_get_capacity, {
                cursor.collect::<Vec<_>>().await
            });

            for r in results {
                if let Ok(num) = r {
                    cache_cap.insert(format!("{}_cap", num.hotel_id), num.number);

                    let key = format!("{}_cap", num.hotel_id);
                    let value = num.number.to_string();
                    let mc_timeout_resp = track_latency!(self.check_avail_stats.mc_set_capacity, {
                        tokio::time::timeout(
                            MC_TIMEOUT,
                            mc_client.set(&key, value.as_bytes(), None, None),
                        )
                        .await
                    });
                    if let Ok(resp) = mc_timeout_resp {
                        if is_mc_protocol_err(&resp) {
                            // log::error!("CheckAvail hotel cap writeback should succeed");
                            mc_client = mc_client.replace().await;
                        }
                    }
                }
            }
        }

        // Process date ranges and create queries
        let mut query_map = HashMap::new();
        let mut req_commands = Vec::new();

        for hotel_id in &req.hotel_ids {
            let in_date =
                DateTime::parse_from_rfc3339(&format!("{}T12:00:00+00:00", req.in_date)).unwrap();
            let out_date =
                DateTime::parse_from_rfc3339(&format!("{}T12:00:00+00:00", req.out_date)).unwrap();

            let mut current_date = in_date;
            while current_date < out_date {
                let in_date_str = current_date.format("%Y-%m-%d").to_string();
                current_date = current_date + chrono::Duration::days(1);
                let out_date_str = current_date.format("%Y-%m-%d").to_string();

                let memc_key = format!("{}_{}_{}", hotel_id, in_date_str, out_date_str);

                req_commands.push(memc_key.clone());
                query_map.insert(memc_key, (hotel_id.clone(), in_date_str, out_date_str));
            }
        }

        self.check_avail_mc_reserve.track(req_commands.len());
        let mc_resp = track_latency!(self.check_avail_stats.mc_get_reservations, {
            tokio::time::timeout(MC_TIMEOUT, mc_client.get_multi(req_commands)).await
        });
        if let Ok(Ok(mc_resp)) = mc_resp {
            for entry in mc_resp {
                let key = String::from_utf8(entry.key).unwrap();
                query_map.remove(&key).map(|(hotel_id, _, _)| {
                    if let Ok(count) = String::from_utf8(entry.data.unwrap())
                        .unwrap_or_default()
                        .parse::<i32>()
                    {
                        let cap = cache_cap.get(&format!("{}_cap", hotel_id)).unwrap_or(&0);
                        if count + req.room_number > *cap {
                            res_map
                                .entry(hotel_id.to_owned())
                                .and_modify(|e| *e = false);
                        }
                    }
                });
            }
        }

        // Check reservations in parallel
        let mut tasks = Vec::new();

        // let query_lim = 1;
        // self.check_avail_reserve
        //     .track(std::cmp::min(query_map.len(), query_lim));
        self.check_avail_mongo_reserve.track(query_map.len());
        // for (command, (hotel_id, start_date, end_date)) in query_map.into_iter().take(query_lim) {
        for (command, (hotel_id, start_date, end_date)) in query_map {
            let hotel_id = hotel_id.to_owned();
            let start_date = start_date.to_owned();
            let end_date = end_date.to_owned();

            let pool = self.mc_pool.clone();
            let mongo_client = self.mongo_client.clone();
            let cache_cap = cache_cap.clone();
            let room_number = req.room_number;
            let mc_err = self.mc_err_count.clone();
            let check_avail_stats = self.check_avail_stats.clone();
            tasks.push(tokio::spawn(async move {
                let collection = mongo_client
                    .database("reservation-db")
                    .collection::<db::Reservation>("reservation");

                let filter = doc! {
                    "hotelId": hotel_id.clone(),
                    "inDate": start_date,
                    "outDate": end_date
                };

                let mut mc = pool.get().await;
                if let Ok(cursor) = collection.find(filter, None).await {
                    let results = track_latency!(check_avail_stats.mongo_get_reservations, {
                        cursor.collect::<Vec<_>>().await
                    });

                    let mut count = 0;
                    for r in results {
                        if let Ok(reservation) = r {
                            count += reservation.number;
                        }
                    }

                    // Update memcached

                    let mc_timeout_res = track_latency!(check_avail_stats.mc_set_reservations, {
                        tokio::time::timeout(
                            MC_TIMEOUT,
                            mc.set(&command, count.to_string().as_bytes(), None, None),
                        )
                        .await
                    });
                    if let Ok(mc_res) = mc_timeout_res {
                        if mc_res.is_err() {
                            mc_err.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    let hid = hotel_id.clone();
                    let cap = cache_cap.get(&format!("{}_cap", hid)).unwrap_or(&0);
                    if count + room_number > *cap {
                        return (hotel_id, false);
                    }
                }
                (hotel_id, true)
            }));
        }

        // Wait for all tasks to complete
        for task in tasks {
            let (hotel_id, is_available) = task.await.unwrap();
            res_map.insert(hotel_id, is_available);
        }

        // Collect results
        let mut resp = reservation::ReservationResponse {
            hotel_ids: Vec::new(),
        };
        for (hotel_id, available) in res_map {
            if available {
                resp.hotel_ids.push(hotel_id);
            }
        }

        {
            let elapsed = start.elapsed().as_micros();
            self.check_avail_stats
                .e2e
                .track(elapsed.try_into().unwrap());
        }

        Ok(Response::new(resp))
    }

    async fn make_reservation(
        &self,
        req: Request<reservation::ReservationRequest>,
    ) -> Result<Response<reservation::ReservationResponse>, Status> {
        let start = Instant::now();

        let req = req.into_inner();

        let mut res = reservation::ReservationResponse {
            hotel_ids: Vec::new(),
        };

        let database = self.mongo_reserve_client.database("reservation-db");
        let res_collection: Collection<db::Reservation> = database.collection("reservation");
        let num_collection: Collection<db::Number> = database.collection("number");
        let mut mc_client = self.mc_pool.get().await;

        let in_date = DateTime::parse_from_rfc3339(&format!("{}T12:00:00+00:00", req.in_date))
            .unwrap()
            .with_timezone(&chrono::Utc);
        let out_date = DateTime::parse_from_rfc3339(&format!("{}T12:00:00+00:00", req.out_date))
            .unwrap()
            .with_timezone(&chrono::Utc);

        let hotel_id = &req.hotel_ids[0];
        let mut current_date = in_date;
        let mut memc_date_num_map = HashMap::new();

        let mut iters = 0;
        while current_date < out_date {
            iters += 1;
            current_date = current_date + chrono::Duration::days(1);

            let in_date_str = current_date.format("%Y-%m-%d").to_string();
            let out_date_str = current_date.format("%Y-%m-%d").to_string();
            let memc_key = format!("{}_{}_{}", hotel_id, in_date_str, out_date_str);

            // Check memcached
            let count = match mc_client.get(&memc_key).await {
                Ok(Some(value)) => {
                    // Memcached hit
                    let count = String::from_utf8_lossy(&value.data.unwrap())
                        .parse::<i32>()
                        .unwrap();
                    memc_date_num_map.insert(memc_key, count + req.room_number);
                    count
                }
                _ => {
                    // Memcached miss
                    let filter = doc! {
                        "hotelId": hotel_id,
                        "inDate": &in_date_str,
                        "outDate": &out_date_str
                    };

                    let mut reservations = res_collection.find(filter, None).await.unwrap();

                    let mut reserve_count = 0;
                    use futures::StreamExt;
                    while let Some(reservation) = reservations.next().await {
                        reserve_count += reservation.unwrap().number;
                    }
                    memc_date_num_map.insert(memc_key, reserve_count + req.room_number);
                    reserve_count
                }
            };

            // Check capacity
            let memc_cap_key = format!("{}_cap", hotel_id);
            let hotel_cap = match mc_client.get(&memc_cap_key).await {
                Ok(Some(value)) => String::from_utf8_lossy(&value.data.unwrap())
                    .parse::<i32>()
                    .unwrap(),
                _ => {
                    let filter = doc! { "hotelId": hotel_id };
                    let num = num_collection
                        .find_one(filter, None)
                        .await
                        .unwrap()
                        .expect(&format!("should find hotel {}", hotel_id));

                    let cap = num.number;
                    if mc_client
                        .set(&memc_cap_key, cap.to_string().as_bytes(), None, None)
                        .await
                        .is_err()
                    {
                        self.mc_err_count.fetch_add(1, Ordering::Relaxed);
                    }
                    cap
                }
            };

            if count + req.room_number > hotel_cap {
                return Ok(Response::new(res));
            }
        }
        self.mk_reserve_mongo.track(iters);

        // Update reservation number cache
        let kvs: Vec<_> = memc_date_num_map
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect();
        let mc_kvs: Vec<_> = kvs.iter().map(|(k, v)| (k, v)).collect();
        mc_client.set_multi(&mc_kvs, None, None).await.ok();

        // Insert reservations
        let mut reservations = Vec::new();
        let mut current_date = in_date;
        while current_date < out_date {
            current_date = current_date + chrono::Duration::days(1);
            let in_date_str = current_date.format("%Y-%m-%d").to_string();
            let out_date_str = current_date.format("%Y-%m-%d").to_string();

            let reservation = db::Reservation {
                hotel_id: hotel_id.clone(),
                customer_name: req.customer_name.clone(),
                in_date: in_date_str,
                out_date: out_date_str,
                number: req.room_number,
            };
            reservations.push(reservation);
        }
        res_collection
            .insert_many(reservations, None)
            .await
            .unwrap();
        res.hotel_ids.push(hotel_id.clone());

        // {
        //     let elapsed = start.elapsed().as_micros();
        //     self.lat_make_reserve.lock().unwrap().track(elapsed as u64);
        // }

        Ok(Response::new(res))
    }
}
