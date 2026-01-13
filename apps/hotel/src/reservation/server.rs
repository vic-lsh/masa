pub mod hotel_tonic {
    pub mod reservation {
        tonic::include_proto!("reservation");
    }
}
use app_utils::stats::AvgTracker;
use chrono::DateTime;
use hotel_tonic::reservation::{self, reservation_server::Reservation};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::config::ReservationConfig;
use crate::db;
use app_util_macros::track_latency;
use app_utils::stats::latency::StatsTracker;
use mongodb::{bson::doc, Client as MongoClient, Collection};
use redis::{aio::ConnectionManager as RedisConnectionManager, AsyncCommands};
use tonic::{Request, Response, Status};

const TIMING_LOG_EVERY_SECS: u64 = 3;

struct CheckAvailTiming {
    last_log: Instant,
    count: u64,
    cap_redis_us: u64,
    cap_mongo_us: u64,
    build_query_us: u64,
    reserve_redis_us: u64,
    mongo_reserve_us: u64,
    collect_us: u64,
    total_us: u64,
}

impl CheckAvailTiming {
    fn new() -> Self {
        Self {
            last_log: Instant::now(),
            count: 0,
            cap_redis_us: 0,
            cap_mongo_us: 0,
            build_query_us: 0,
            reserve_redis_us: 0,
            mongo_reserve_us: 0,
            collect_us: 0,
            total_us: 0,
        }
    }
}

static CHECK_AVAIL_TIMING: OnceLock<Mutex<CheckAvailTiming>> = OnceLock::new();

fn record_check_avail_timing(
    cap_redis: Duration,
    cap_mongo: Duration,
    build_query: Duration,
    reserve_redis: Duration,
    mongo_reserve: Duration,
    collect: Duration,
    total: Duration,
) {
    let timing = CHECK_AVAIL_TIMING.get_or_init(|| Mutex::new(CheckAvailTiming::new()));
    let mut timing = timing.lock().unwrap();
    timing.count += 1;
    timing.cap_redis_us += cap_redis.as_micros() as u64;
    timing.cap_mongo_us += cap_mongo.as_micros() as u64;
    timing.build_query_us += build_query.as_micros() as u64;
    timing.reserve_redis_us += reserve_redis.as_micros() as u64;
    timing.mongo_reserve_us += mongo_reserve.as_micros() as u64;
    timing.collect_us += collect.as_micros() as u64;
    timing.total_us += total.as_micros() as u64;

    if timing.last_log.elapsed() >= Duration::from_secs(TIMING_LOG_EVERY_SECS) && timing.count > 0 {
        let count = timing.count;
        log::info!(
            "check_availability avg timing ({} req): cap_redis={}us cap_mongo={}us build_query={}us reserve_redis={}us mongo_reserve={}us collect={}us total={}us",
            count,
            timing.cap_redis_us / count,
            timing.cap_mongo_us / count,
            timing.build_query_us / count,
            timing.reserve_redis_us / count,
            timing.mongo_reserve_us / count,
            timing.collect_us / count,
            timing.total_us / count,
        );
        *timing = CheckAvailTiming::new();
    }
}

pub struct ReservationImpl {
    redis_conn: RedisConnectionManager,
    mongo_client: MongoClient,
    mongo_reserve_client: MongoClient,
    check_avail_redis_hotel_cap: Arc<AvgTracker>,
    check_avail_mongo_hotel_cap: Arc<AvgTracker>,
    check_avail_redis_reserve: Arc<AvgTracker>,
    check_avail_mongo_reserve: Arc<AvgTracker>,
    mk_reserve_mongo: Arc<AvgTracker>,
    redis_err_count: Arc<AtomicUsize>,

    check_avail_stats: Arc<StatsTracker>,
}

impl ReservationImpl {
    pub async fn new(config: ReservationConfig) -> Result<Self, Box<dyn Error>> {
        let redis_client = redis::Client::open(config.redis_addr.as_str())?;
        let redis_conn = RedisConnectionManager::new(redis_client).await?;
        let mongo_client = crate::db::initialize_database(&config.mongodb_addr).await?;
        let client_options = mongodb::options::ClientOptions::parse(&config.mongodb_addr).await?;
        let mongo_reserve_client = MongoClient::with_options(client_options)?;

        let check_avail_redis_hotel_cap = Arc::new(AvgTracker::default());
        let check_avail_mongo_hotel_cap = Arc::new(AvgTracker::default());

        let check_avail_redis_reserve = Arc::new(AvgTracker::default());
        let check_avail_mongo_reserve = Arc::new(AvgTracker::default());

        let mk_reserve_mongo = Arc::new(AvgTracker::default());

        let redis_err_count = Arc::new(AtomicUsize::new(0));

        Ok(Self {
            redis_conn,
            mongo_client,
            mongo_reserve_client,
            check_avail_redis_reserve,
            check_avail_mongo_reserve,
            check_avail_redis_hotel_cap,
            check_avail_mongo_hotel_cap,
            mk_reserve_mongo,
            redis_err_count,

            check_avail_stats: Arc::new(StatsTracker::new(
                vec![
                    "e2e",
                    "redis_get_capacity",
                    "mongo_get_capacity",
                    "redis_set_capacity",
                    "redis_get_reservations",
                    "mongo_get_reservations",
                    "redis_set_reservations",
                ],
                true,
            )),
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

        let timing_start = Instant::now();

        // even with this the app still hangs under high load...
        // scheduler bug?
        const REDIS_TIMEOUT: Duration = Duration::from_millis(100);

        let start = Instant::now();

        let req = req.into_inner();

        // Create hotel memory keys and maps
        let mut hotel_mem_keys = Vec::new();
        let mut missing_keys = HashSet::new();
        let mut res_map: HashMap<String, bool> = HashMap::new();

        self.check_avail_redis_hotel_cap.track(req.hotel_ids.len());
        for hotel_id in &req.hotel_ids {
            let cap_key = format!("{}_cap", hotel_id);
            hotel_mem_keys.push(cap_key.clone());
            missing_keys.insert(cap_key);
            res_map.insert(hotel_id.clone(), true);
        }

        // Get capacity from redis
        let mut cache_cap = HashMap::new();

        let mut redis_conn = self.redis_conn.clone();

        let cap_redis_start = Instant::now();
        let redis_resp = track_latency!(self.check_avail_stats.get("redis_get_capacity"), {
            tokio::time::timeout(
                REDIS_TIMEOUT,
                redis::cmd("MGET")
                    .arg(&hotel_mem_keys)
                    .query_async::<_, Vec<Option<Vec<u8>>>>(&mut redis_conn),
            )
            .await
        });
        let cap_redis_elapsed = cap_redis_start.elapsed();
        if let Ok(Ok(redis_resp)) = redis_resp {
            for (key, value) in hotel_mem_keys.iter().zip(redis_resp) {
                if let Some(raw) = value {
                    if let Ok(cap) = String::from_utf8(raw).unwrap_or_default().parse::<i32>() {
                        missing_keys.remove(key);
                        cache_cap.insert(key.clone(), cap);
                    }
                }
            }
        } else {
            self.redis_err_count.fetch_add(1, Ordering::Relaxed);
        }

        // let max_missing_keys = 2;
        // let mut missing_keys: HashSet<_> = missing_keys.drain().take(max_missing_keys).collect();
        self.check_avail_mongo_hotel_cap.track(missing_keys.len());
        // Handle cache misses with MongoDB
        let mut cap_mongo_elapsed = Duration::from_micros(0);
        if !missing_keys.is_empty() {
            let cap_mongo_start = Instant::now();
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

            let results = track_latency!(self.check_avail_stats.get("mongo_get_capacity"), {
                cursor.collect::<Vec<_>>().await
            });

            for r in results {
                if let Ok(num) = r {
                    cache_cap.insert(format!("{}_cap", num.hotel_id), num.number);

                    let key = format!("{}_cap", num.hotel_id);
                    let value = num.number.to_string();
                    let redis_timeout_resp =
                        track_latency!(self.check_avail_stats.get("redis_set_capacity"), {
                            tokio::time::timeout(
                                REDIS_TIMEOUT,
                                redis_conn.set::<_, _, ()>(&key, value.clone()),
                            )
                            .await
                        });
                    if redis_timeout_resp.is_err() {
                        self.redis_err_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            cap_mongo_elapsed = cap_mongo_start.elapsed();
        }

        // Process date ranges and create queries
        let build_query_start = Instant::now();
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

                let redis_key = format!("{}_{}_{}", hotel_id, in_date_str, out_date_str);

                req_commands.push(redis_key.clone());
                query_map.insert(redis_key, (hotel_id.clone(), in_date_str, out_date_str));
            }
        }
        let build_query_elapsed = build_query_start.elapsed();

        self.check_avail_redis_reserve.track(req_commands.len());
        let reserve_redis_start = Instant::now();
        let redis_resp = track_latency!(self.check_avail_stats.get("redis_get_reservations"), {
            tokio::time::timeout(
                REDIS_TIMEOUT,
                redis::cmd("MGET")
                    .arg(&req_commands)
                    .query_async::<_, Vec<Option<Vec<u8>>>>(&mut redis_conn),
            )
            .await
        });
        let reserve_redis_elapsed = reserve_redis_start.elapsed();

        if let Ok(Ok(redis_resp)) = redis_resp {
            for (key, value) in req_commands.iter().zip(redis_resp) {
                if let Some(raw) = value {
                    if let Some((hotel_id, _, _)) = query_map.remove(key) {
                        if let Ok(count) = String::from_utf8(raw).unwrap_or_default().parse::<i32>()
                        {
                            let cap = cache_cap.get(&format!("{}_cap", hotel_id)).unwrap_or(&0);
                            if count + req.room_number > *cap {
                                res_map
                                    .entry(hotel_id.to_owned())
                                    .and_modify(|e| *e = false);
                            }
                        }
                    }
                }
            }
        } else {
            self.redis_err_count.fetch_add(1, Ordering::Relaxed);
        }

        // Check reservations in parallel
        let mongo_reserve_start = Instant::now();
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

            let redis_conn = self.redis_conn.clone();
            let mongo_client = self.mongo_client.clone();
            let cache_cap = cache_cap.clone();
            let room_number = req.room_number;
            let redis_err = self.redis_err_count.clone();
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

                let mut redis_conn = redis_conn.clone();
                if let Ok(cursor) = collection.find(filter, None).await {
                    let results =
                        track_latency!(check_avail_stats.get("mongo_get_reservations"), {
                            cursor.collect::<Vec<_>>().await
                        });

                    let mut count = 0;
                    for r in results {
                        if let Ok(reservation) = r {
                            count += reservation.number;
                        }
                    }

                    // Update redis
                    let redis_timeout_res =
                        track_latency!(check_avail_stats.get("redis_set_reservations"), {
                            tokio::time::timeout(
                                REDIS_TIMEOUT,
                                redis_conn.set::<_, _, ()>(&command, count.to_string()),
                            )
                            .await
                        });
                    if redis_timeout_res.is_err() {
                        redis_err.fetch_add(1, Ordering::Relaxed);
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
        let mongo_reserve_elapsed = mongo_reserve_start.elapsed();

        // Collect results
        let collect_start = Instant::now();
        let mut resp = reservation::ReservationResponse {
            hotel_ids: Vec::new(),
        };
        for (hotel_id, available) in res_map {
            if available {
                resp.hotel_ids.push(hotel_id);
            }
        }
        let collect_elapsed = collect_start.elapsed();

        record_check_avail_timing(
            cap_redis_elapsed,
            cap_mongo_elapsed,
            build_query_elapsed,
            reserve_redis_elapsed,
            mongo_reserve_elapsed,
            collect_elapsed,
            timing_start.elapsed(),
        );

        tokio::time::sleep(Duration::from_millis(100)).await;

        {
            let elapsed = start.elapsed().as_micros();
            self.check_avail_stats
                .get("e2e")
                .track(elapsed.try_into().unwrap());
        }

        Ok(Response::new(resp))
    }

    async fn make_reservation(
        &self,
        req: Request<reservation::ReservationRequest>,
    ) -> Result<Response<reservation::ReservationResponse>, Status> {
        let req = req.into_inner();

        let mut res = reservation::ReservationResponse {
            hotel_ids: Vec::new(),
        };

        let database = self.mongo_reserve_client.database("reservation-db");
        let res_collection: Collection<db::Reservation> = database.collection("reservation");
        let num_collection: Collection<db::Number> = database.collection("number");
        let mut redis_conn = self.redis_conn.clone();

        let in_date = DateTime::parse_from_rfc3339(&format!("{}T12:00:00+00:00", req.in_date))
            .unwrap()
            .with_timezone(&chrono::Utc);
        let out_date = DateTime::parse_from_rfc3339(&format!("{}T12:00:00+00:00", req.out_date))
            .unwrap()
            .with_timezone(&chrono::Utc);

        let hotel_id = &req.hotel_ids[0];
        let mut current_date = in_date;
        let mut redis_date_num_map = HashMap::new();

        let mut iters = 0;
        while current_date < out_date {
            iters += 1;
            let next_date = current_date + chrono::Duration::days(1);

            let in_date_str = current_date.format("%Y-%m-%d").to_string();
            let out_date_str = next_date.format("%Y-%m-%d").to_string();
            let redis_key = format!("{}_{}_{}", hotel_id, in_date_str, out_date_str);

            // Check redis
            let count = match redis_conn.get::<_, Option<Vec<u8>>>(&redis_key).await {
                Ok(Some(value)) => {
                    // Redis hit
                    let count = String::from_utf8_lossy(&value).parse::<i32>().unwrap();
                    redis_date_num_map.insert(redis_key, count + req.room_number);
                    count
                }
                _ => {
                    // Redis miss
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
                    redis_date_num_map.insert(redis_key, reserve_count + req.room_number);
                    reserve_count
                }
            };

            // Check capacity
            let redis_cap_key = format!("{}_cap", hotel_id);
            let hotel_cap = match redis_conn.get::<_, Option<Vec<u8>>>(&redis_cap_key).await {
                Ok(Some(value)) => String::from_utf8_lossy(&value).parse::<i32>().unwrap(),
                _ => {
                    let filter = doc! { "hotelId": hotel_id };
                    let num = num_collection
                        .find_one(filter, None)
                        .await
                        .unwrap()
                        .expect(&format!("should find hotel {}", hotel_id));

                    let cap = num.number;
                    if redis_conn
                        .set::<_, _, ()>(&redis_cap_key, cap.to_string())
                        .await
                        .is_err()
                    {
                        self.redis_err_count.fetch_add(1, Ordering::Relaxed);
                    }
                    cap
                }
            };

            if count + req.room_number > hotel_cap {
                return Ok(Response::new(res));
            }

            current_date = next_date;
        }
        self.mk_reserve_mongo.track(iters);

        // Update reservation number cache
        let kvs: Vec<_> = redis_date_num_map
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect();
        for (key, value) in kvs {
            if redis_conn
                .set::<_, _, ()>(&key, value.clone())
                .await
                .is_err()
            {
                self.redis_err_count.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Insert reservations
        let mut reservations = Vec::new();
        let mut current_date = in_date;
        while current_date < out_date {
            let next_date = current_date + chrono::Duration::days(1);
            let in_date_str = current_date.format("%Y-%m-%d").to_string();
            let out_date_str = next_date.format("%Y-%m-%d").to_string();

            let reservation = db::Reservation {
                hotel_id: hotel_id.clone(),
                customer_name: req.customer_name.clone(),
                in_date: in_date_str,
                out_date: out_date_str,
                number: req.room_number,
            };
            reservations.push(reservation);
            current_date = next_date;
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
