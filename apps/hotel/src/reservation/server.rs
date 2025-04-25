pub mod hotel_tonic {
    pub mod reservation {
        tonic::include_proto!("reservation");
    }
}
use chrono::DateTime;
use hotel::AvgTracker;
use masa::LatencyTracker;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::db;
use hotel::McPool;
use mongodb::{bson::doc, Client as MongoClient, Collection};
use tonic::{Request, Response, Status};

use hotel_tonic::{reservation, reservation::reservation_server::Reservation};

pub struct ReservationImpl {
    mc_pool: Arc<McPool>,
    mongo_client: Arc<MongoClient>,
    lat_check_avail: Mutex<LatencyTracker>,
    lat_make_reserve: Mutex<LatencyTracker>,
    check_avail_hotel_mc: Arc<AvgTracker>,
    check_avail_hotel_mongo: Arc<AvgTracker>,
    check_avail_reserve: Arc<AvgTracker>,
    check_reserve: Arc<AvgTracker>,
}

impl ReservationImpl {
    pub async fn new(cache_addr: String, db_addr: String) -> Result<Self, Box<dyn Error>> {
        let mongo_client = crate::db::initialize_database(&db_addr).await?;
        let lat_check_avail = Mutex::new(LatencyTracker::new("check_availability".into(), 256));
        let lat_make_reserve = Mutex::new(LatencyTracker::new("make_reservation".into(), 256));

        let check_avail_hotel_mc = Arc::new(AvgTracker::default());
        let check_avail_hotel_mongo = Arc::new(AvgTracker::default());
        let check_avail_reserve = Arc::new(AvgTracker::default());
        let check_reserve = Arc::new(AvgTracker::default());

        let ca_hotel_mc = check_avail_hotel_mc.clone();
        let ca_hotel_mongo = check_avail_hotel_mongo.clone();
        let ca_reserve = check_avail_reserve.clone();
        let reserve = check_reserve.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                log::info!(
                    "CheckAvail: mc {} mongo {} reserve {}; MkReserve: {}",
                    ca_hotel_mc.get(),
                    ca_hotel_mongo.get(),
                    ca_reserve.get(),
                    reserve.get()
                );
            }
        });

        let cache_addr = cache_addr
            .strip_prefix("memcache://")
            .map(|addr| format!("tcp://{}", addr))
            .unwrap()
            .to_owned();

        Ok(Self {
            mc_pool: Arc::new(McPool::new(cache_addr, 128)),
            mongo_client: Arc::new(mongo_client),
            lat_check_avail,
            lat_make_reserve,
            check_avail_reserve,
            check_avail_hotel_mc,
            check_avail_hotel_mongo,
            check_reserve,
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

        let start = Instant::now();

        let req = req.into_inner();

        // Create hotel memory keys and maps
        let mut hotel_mem_keys = Vec::new();
        let mut missing_keys = HashSet::new();
        let mut res_map: HashMap<String, bool> = HashMap::new();

        self.check_avail_hotel_mc.track(req.hotel_ids.len());
        for hotel_id in &req.hotel_ids {
            let cap_key = format!("{}_cap", hotel_id);
            hotel_mem_keys.push(cap_key.clone());
            missing_keys.insert(cap_key);
            res_map.insert(hotel_id.clone(), true);
        }

        // Get capacity from memcached
        let mut cache_cap = HashMap::new();

        let mut mc_client = self.mc_pool.get().await;
        // let mut mc_client = async_memcached::Client::new(&self.mc_pool.addr)
        //     .await
        //     .unwrap();

        if let Ok(mc_resp) = mc_client.get_multi(hotel_mem_keys).await {
            for entry in mc_resp {
                if let Ok(cap) = String::from_utf8(entry.data)
                    .unwrap_or_default()
                    .parse::<i32>()
                {
                    let hotel_id = String::from_utf8(entry.key).unwrap();
                    missing_keys.remove(&hotel_id);
                    cache_cap.insert(hotel_id, cap);
                }
            }
        }

        self.check_avail_hotel_mongo.track(missing_keys.len());
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

            let mut cursor = num_collection
                .find(doc! { "hotelId": { "$in": &query_miss_keys } }, None)
                .await
                .map_err(|e| tonic::Status::internal(format!("mongo error: {}", e)))?;

            while let Some(doc) = cursor.next().await {
                if let Ok(num) = doc {
                    cache_cap.insert(format!("{}_cap", num.hotel_id), num.number);

                    // Async set to memcached
                    let key = format!("{}_cap", num.hotel_id);
                    let value = num.number.to_string();
                    // let pool = self.mc_pool.clone();
                    // async_executor::spawn(async move {
                    //     let mut mc = pool.get().await;
                    //     mc.set(&key, value.as_bytes(), None, None).await.ok();
                    // })
                    // .detach();
                    mc_client.set(&key, value.as_bytes(), None, None).await.ok();
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

                let memc_key = format!("{}_{}_{}", hotel_id, out_date_str, out_date_str);

                req_commands.push(memc_key.clone());
                query_map.insert(memc_key, (hotel_id.clone(), in_date_str, out_date_str));
            }
        }

        if let Ok(mc_resp) = mc_client.get_multi(req_commands).await {
            for entry in mc_resp {
                let key = String::from_utf8(entry.key).unwrap();
                query_map.remove(&key).map(|(hotel_id, _, _)| {
                    if let Ok(count) = String::from_utf8(entry.data)
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

        self.check_avail_reserve.track(query_map.len());
        for (command, (hotel_id, start_date, end_date)) in query_map {
            let hotel_id = hotel_id.to_owned();
            let start_date = start_date.to_owned();
            let end_date = end_date.to_owned();

            let pool = self.mc_pool.clone();
            // let pool_addr = self.mc_pool.addr.clone();
            let mongo_client = self.mongo_client.clone();
            let cache_cap = cache_cap.clone();
            let room_number = req.room_number;
            tasks.push(async_executor::spawn(async move {
                let collection = mongo_client
                    .database("reservation-db")
                    .collection::<db::Reservation>("reservation");

                let filter = doc! {
                    "hotelId": hotel_id.clone(),
                    "inDate": start_date,
                    "outDate": end_date
                };

                if let Ok(mut cursor) = collection.find(filter, None).await {
                    let mut count = 0;
                    while let Some(Ok(reservation)) = cursor.next().await {
                        count += reservation.number;
                    }

                    // let mut mc = async_memcached::Client::new(&pool_addr).await.unwrap();
                    let mut mc = pool.get().await;
                    // Update memcached
                    mc.set(&command, count.to_string().as_bytes(), None, None)
                        .await
                        .ok();

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
            let (hotel_id, is_available) = task.await;
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
            self.lat_check_avail.lock().unwrap().track(elapsed as u64);
        }

        Ok(Response::new(resp))
    }

    async fn make_reservation(
        &self,
        req: Request<reservation::ReservationRequest>,
    ) -> Result<Response<reservation::ReservationResponse>, Status> {
        // // let ctx = request.metadata().get_ctx("ctx").unwrap();
        // let request = request.into_inner();
        // // [NOTE] The original implementation only processes the first hotel.
        // assert!(request.hotels.len() == 1);
        // let response = self.check_availability(request.clone()).await;
        // // [NOTE] Optional multi-threading.
        // for hotel in &response.hotels {
        //     for date in request.in_date..request.out_date {
        //         self.manager
        //             .update_availability(hotel, date, request.num_rooms)
        //             .await;
        //     }
        // }
        // Ok(Response::new(response))

        let start = Instant::now();

        let req = req.into_inner();

        let mut res = reservation::ReservationResponse {
            hotel_ids: Vec::new(),
        };

        let database = self.mongo_client.database("reservation-db");
        let res_collection: Collection<db::Reservation> = database.collection("reservation");
        let num_collection: Collection<db::Number> = database.collection("number");

        let mut mc_client = self.mc_pool.get().await;
        // let mut mc_client = async_memcached::Client::new(&self.mc_pool.addr)
        //     .await
        //     .unwrap();

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
                    let count = String::from_utf8_lossy(&value.data).parse::<i32>().unwrap();
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
                Ok(Some(value)) => String::from_utf8_lossy(&value.data).parse::<i32>().unwrap(),
                _ => {
                    let filter = doc! { "hotelId": hotel_id };
                    let num = num_collection
                        .find_one(filter, None)
                        .await
                        .unwrap()
                        .expect(&format!("should find hotel {}", hotel_id));

                    let cap = num.number;
                    mc_client
                        .set(&memc_cap_key, cap.to_string().as_bytes(), None, None)
                        .await
                        .ok();
                    cap
                }
            };

            if count + req.room_number > hotel_cap {
                return Ok(Response::new(res));
            }
        }
        self.check_reserve.track(iters);

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

        {
            let elapsed = start.elapsed().as_micros();
            self.lat_make_reserve.lock().unwrap().track(elapsed as u64);
        }

        Ok(Response::new(res))
    }
}
