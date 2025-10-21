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
use std::io::{self, ErrorKind};
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{error, warn};

use crate::config::ReservationConfig;
use crate::db;
use app_util_macros::track_latency;
use app_utils::pool::{Pool, PoolItemRef};
use app_utils::stats::latency::StatsTracker;
use memcache_async::ascii::Protocol as McProtocol;
use mongodb::{bson::doc, Client as MongoClient, Collection};
use tokio::net::TcpStream;
use tonic::{Request, Response, Status};

type McClient = McProtocol<TcpStream>;
const MC_POOL_MAX_SIZE: usize = 256;

async fn connect_memcache(addr: &str) -> io::Result<McClient> {
    let stream = TcpStream::connect(addr).await?;
    stream.set_nodelay(true)?;
    Ok(McProtocol::new(stream))
}

pub struct ReservationImpl {
    cache_addr: String,
    mc_pool: Arc<Pool<McClient>>,
    mongo_client: MongoClient,
    mongo_reserve_client: MongoClient,
    check_avail_mc_hotel_cap: Arc<AvgTracker>,
    check_avail_mongo_hotel_cap: Arc<AvgTracker>,
    check_avail_mc_reserve: Arc<AvgTracker>,
    check_avail_mongo_reserve: Arc<AvgTracker>,
    mk_reserve_mongo: Arc<AvgTracker>,
    mc_err_count: Arc<AtomicUsize>,

    check_avail_stats: Arc<StatsTracker>,
}

impl ReservationImpl {
    pub async fn new(config: ReservationConfig) -> Result<Self, Box<dyn Error>> {
        let mongo_client = crate::db::initialize_database(&config.mongodb_addr).await?;
        let client_options = mongodb::options::ClientOptions::parse(&config.mongodb_addr).await?;
        let mongo_reserve_client = MongoClient::with_options(client_options)?;

        let check_avail_mc_hotel_cap = Arc::new(AvgTracker::default());
        let check_avail_mongo_hotel_cap = Arc::new(AvgTracker::default());

        let check_avail_mc_reserve = Arc::new(AvgTracker::default());
        let check_avail_mongo_reserve = Arc::new(AvgTracker::default());

        let mk_reserve_mongo = Arc::new(AvgTracker::default());

        let mc_err_count = Arc::new(AtomicUsize::new(0));

        let cache_addr = config
            .memcached_addr
            .strip_prefix("memcache://")
            .or_else(|| config.memcached_addr.strip_prefix("tcp://"))
            .unwrap_or(&config.memcached_addr)
            .to_owned();
        let mc_pool = Arc::new(Pool::new(MC_POOL_MAX_SIZE));

        Ok(Self {
            cache_addr,
            mc_pool,
            mongo_client,
            mongo_reserve_client,
            check_avail_mc_reserve,
            check_avail_mongo_reserve,
            check_avail_mc_hotel_cap,
            check_avail_mongo_hotel_cap,
            mk_reserve_mongo,
            mc_err_count,

            check_avail_stats: Arc::new(StatsTracker::new(
                vec![
                    "e2e",
                    "mc_get_capacity",
                    "mongo_get_capacity",
                    "mc_set_capacity",
                    "mc_get_reservations",
                    "mongo_get_reservations",
                    "mc_set_reservations",
                ],
                true,
            )),
        })
    }

    async fn get_mc_client(&self) -> Result<PoolItemRef<'_, McClient>, Status> {
        let cache_addr = self.cache_addr.clone();
        match self
            .mc_pool
            .try_get_or_create_async({
                let cache_addr = cache_addr.clone();
                move || {
                    let cache_addr = cache_addr.clone();
                    async move { connect_memcache(&cache_addr).await }
                }
            })
            .await
        {
            Ok(client) => Ok(client),
            Err(err) => {
                error!("mc_connect error for {}: {:?}", cache_addr, err);
                Err(Status::internal(format!(
                    "memcache connection failed: {}",
                    err
                )))
            }
        }
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

        {
            let mut mc_client = self.get_mc_client().await?;
            let mc_resp = track_latency!(self.check_avail_stats.get("mc_get_capacity"), {
                tokio::time::timeout(MC_TIMEOUT, mc_client.get_multi(hotel_mem_keys.as_slice()))
                    .await
            });
            match mc_resp {
                Ok(Ok(mc_values)) => {
                    for (key, value) in mc_values {
                        if let Ok(cap_str) = String::from_utf8(value) {
                            if let Ok(cap) = cap_str.parse::<i32>() {
                                missing_keys.remove(&key);
                                cache_cap.insert(key, cap);
                            }
                        }
                    }
                }
                Ok(Err(err)) => {
                    error!("mc_get_capacity error: {:?}", err);
                    mc_client.discard();
                }
                Err(err) => {
                    error!("mc_get_capacity timeout: {:?}", err);
                    mc_client.discard();
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

            let results = track_latency!(self.check_avail_stats.get("mongo_get_capacity"), {
                cursor.collect::<Vec<_>>().await
            });

            let ok_results = results
                .into_iter()
                .filter(|r| r.is_ok())
                .map(|r| r.unwrap())
                .collect::<Vec<_>>();

            let hotel_ids = ok_results
                .iter()
                .map(|r| r.hotel_id.clone())
                .collect::<Vec<_>>();
            warn!(
                "Missing hotel cap keys {}, {:?}",
                hotel_ids.len(),
                hotel_ids
            );

            let kvs = ok_results
                .into_iter()
                .map(|result| {
                    let key = format!("{}_cap", result.hotel_id);
                    let value = result.number.to_string();
                    (key, value)
                })
                .collect::<Vec<_>>();

            for (key, value) in &kvs {
                if let Ok(cap) = value.parse::<i32>() {
                    cache_cap.insert(key.clone(), cap);
                }
            }

            for (key, value) in &kvs {
                let mut mc_client = self.get_mc_client().await?;
                let mc_timeout_resp =
                    track_latency!(self.check_avail_stats.get("mc_set_capacity"), {
                        tokio::time::timeout(MC_TIMEOUT, mc_client.set(key, value.as_bytes(), 0))
                            .await
                    });
                match mc_timeout_resp {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => {
                        error!("mc_set_capacity error: {:?}", err);
                        self.mc_err_count.fetch_add(1, Ordering::Relaxed);
                        mc_client.discard();
                    }
                    Err(err) => {
                        error!("mc_set_capacity timeout: {:?}", err);
                        mc_client.discard();
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
        {
            let mut mc_client = self.get_mc_client().await?;
            let mc_resp = track_latency!(self.check_avail_stats.get("mc_get_reservations"), {
                tokio::time::timeout(MC_TIMEOUT, mc_client.get_multi(req_commands.as_slice())).await
            });
            match mc_resp {
                Ok(Ok(mc_values)) => {
                    for (key, value) in mc_values {
                        if let Some((hotel_id, _, _)) = query_map.remove(&key) {
                            if let Some(count) = std::str::from_utf8(&value)
                                .ok()
                                .and_then(|s| s.parse::<i32>().ok())
                            {
                                let cap = cache_cap.get(&format!("{}_cap", hotel_id)).unwrap_or(&0);
                                if count + req.room_number > *cap {
                                    res_map.entry(hotel_id.clone()).and_modify(|e| *e = false);
                                }
                            }
                        }
                    }
                }
                Ok(Err(err)) => {
                    error!("mc_get_reservations error: {:?}", err);
                    mc_client.discard();
                }
                Err(err) => {
                    error!("mc_get_reservations timeout: {:?}", err);
                    mc_client.discard();
                }
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

            let cache_addr = self.cache_addr.clone();
            let mongo_client = self.mongo_client.clone();
            let cache_cap = cache_cap.clone();
            let room_number = req.room_number;
            let mc_err = self.mc_err_count.clone();
            let check_avail_stats = self.check_avail_stats.clone();
            let mc_pool = self.mc_pool.clone();
            tasks.push(tokio::spawn(async move {
                let collection = mongo_client
                    .database("reservation-db")
                    .collection::<db::Reservation>("reservation");

                let filter = doc! {
                    "hotelId": hotel_id.clone(),
                    "inDate": start_date,
                    "outDate": end_date
                };

                let mut mc = match mc_pool
                    .try_get_or_create_async({
                        let cache_addr = cache_addr.clone();
                        move || {
                            let cache_addr = cache_addr.clone();
                            async move { connect_memcache(&cache_addr).await }
                        }
                    })
                    .await
                {
                    Ok(client) => client,
                    Err(err) => {
                        error!(
                            "mc_connect error in check_availability task for {}: {:?}",
                            cache_addr, err
                        );
                        mc_err.fetch_add(1, Ordering::Relaxed);
                        return (hotel_id, true);
                    }
                };
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

                    // Update memcached

                    let count_str = count.to_string();
                    let mc_timeout_res =
                        track_latency!(check_avail_stats.get("mc_set_reservations"), {
                            tokio::time::timeout(
                                MC_TIMEOUT,
                                mc.set(&command, count_str.as_bytes(), 0),
                            )
                            .await
                        });
                    match mc_timeout_res {
                        Ok(Ok(())) => {}
                        Ok(Err(err)) => {
                            error!("mc_set_reservations error: {:?}", err);
                            mc_err.fetch_add(1, Ordering::Relaxed);
                            mc.discard();
                        }
                        Err(err) => {
                            error!("mc_set_reservations timeout: {:?}", err);
                            mc_err.fetch_add(1, Ordering::Relaxed);
                            mc.discard();
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
            let count = {
                let mut mc_client = self.get_mc_client().await?;
                match mc_client.get(&memc_key).await {
                    Ok(value) => {
                        // Memcached hit
                        let count = String::from_utf8(value).unwrap().parse::<i32>().unwrap();
                        memc_date_num_map.insert(memc_key.clone(), count + req.room_number);
                        count
                    }
                    Err(err) => {
                        if err.kind() != ErrorKind::NotFound {
                            error!("mc_get reservation count error: {:?}", err);
                            self.mc_err_count.fetch_add(1, Ordering::Relaxed);
                            mc_client.discard();
                        }
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
                        memc_date_num_map.insert(memc_key.clone(), reserve_count + req.room_number);
                        reserve_count
                    }
                }
            };

            // Check capacity
            let memc_cap_key = format!("{}_cap", hotel_id);
            let hotel_cap = {
                let mut mc_client = self.get_mc_client().await?;
                match mc_client.get(&memc_cap_key).await {
                    Ok(value) => String::from_utf8(value).unwrap().parse::<i32>().unwrap(),
                    Err(err) if err.kind() == ErrorKind::NotFound => {
                        let filter = doc! { "hotelId": hotel_id };
                        let num = num_collection
                            .find_one(filter, None)
                            .await
                            .unwrap()
                            .expect(&format!("should find hotel {}", hotel_id));

                        let cap = num.number;
                        let cap_str = cap.to_string();
                        if let Err(set_err) =
                            mc_client.set(&memc_cap_key, cap_str.as_bytes(), 0).await
                        {
                            error!("mc_set capacity error: {:?}", set_err);
                            self.mc_err_count.fetch_add(1, Ordering::Relaxed);
                            mc_client.discard();
                        }
                        cap
                    }
                    Err(err) => {
                        error!("mc_get capacity error: {:?}", err);
                        self.mc_err_count.fetch_add(1, Ordering::Relaxed);
                        mc_client.discard();

                        let filter = doc! { "hotelId": hotel_id };
                        let num = num_collection
                            .find_one(filter, None)
                            .await
                            .unwrap()
                            .expect(&format!("should find hotel {}", hotel_id));
                        let cap = num.number;
                        if let Ok(mut fresh_client) = self.get_mc_client().await {
                            let cap_str = cap.to_string();
                            if let Err(set_err) =
                                fresh_client.set(&memc_cap_key, cap_str.as_bytes(), 0).await
                            {
                                error!("mc_set capacity error after retry: {:?}", set_err);
                                self.mc_err_count.fetch_add(1, Ordering::Relaxed);
                                fresh_client.discard();
                            }
                        }
                        cap
                    }
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
        for (key, value) in &kvs {
            let mut mc_client = self.get_mc_client().await?;
            if let Err(err) = mc_client.set(key, value.as_bytes(), 0).await {
                error!("mc_set reservation cache error: {:?}", err);
                self.mc_err_count.fetch_add(1, Ordering::Relaxed);
                mc_client.discard();
            }
        }

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
