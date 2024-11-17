pub mod hotel {
    pub mod reservation {
        tonic::include_proto!("reservation");
    }
}
use chrono::DateTime;
use reboot_hotel::AvgTracker;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tonic_masa::LatencyTracker;

use crate::db;
use mongodb::{bson::doc, Client as MongoClient, Collection, Database, IndexModel};
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Uniform};
use serde::{Deserialize, Serialize};
use tonic::{Request, Response, Status};

use hotel::{reservation, reservation::reservation_server::Reservation};

// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct Hotel {
//     name: String,
//     date: u32,
//     key: String,
//     n_reservations: u32,
//     n_capacity: u32,
// }
//
// impl Hotel {
//     pub fn new(name: String, date: u32, n_reservations: u32, n_capacity: u32) -> Self {
//         let key = format!("{}_{}", name, date);
//         Hotel {
//             name,
//             date,
//             key,
//             n_reservations,
//             n_capacity,
//         }
//     }
//
//     pub fn key(&self) -> &String {
//         &self.key
//     }
// }
//
// #[derive(Clone)]
// pub struct HotelManager {
//     rng: Arc<Mutex<StdRng>>,
//     uniform_hotel_avail: Uniform<u32>,
//     uniform_cache_miss: Uniform<u32>,
//     hotels: u32,
//     prob_hotel_avail: u32,
//     dates: u32,
//     cache_conns: u32,
//     prob_cache_miss: u32,
//     memcache: memcache::Client,
//     _database: Database,
//     collection: Collection<Hotel>,
// }
//
// impl HotelManager {
//     pub async fn new(
//         hotels: u32,
//         dates: u32,
//         prob_hotel_avail: u32,
//         cache_addr: String,
//         cache_conns: u32,
//         prob_cache_miss: u32,
//         db_addr: String,
//     ) -> Result<Self, Box<dyn Error>> {
//         let manager = {
//             let seed = 998244353;
//             let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(seed)));
//             let uniform_hotel_avail = Uniform::new(0, 100);
//             let uniform_cache_miss = Uniform::new(0, 100);
//             let memcache = memcache::Client::with_pool_size(cache_addr, cache_conns)?;
//             let client = Client::with_uri_str(db_addr).await?;
//             let database = client.database("sheraton");
//             let collection = database.collection::<Hotel>("collection");
//             collection.delete_many(doc! {}, None).await?;
//             HotelManager {
//                 rng,
//                 uniform_hotel_avail,
//                 uniform_cache_miss,
//                 hotels,
//                 prob_hotel_avail,
//                 dates,
//                 cache_conns,
//                 prob_cache_miss,
//                 memcache,
//                 _database: database,
//                 collection,
//             }
//         };
//
//         let manager_clone = manager.clone();
//         let cache = tokio::spawn(async move {
//             log::warn!("Populating Memcached...");
//             manager_clone
//                 .populate_memcache()
//                 .await
//                 .expect("Failed to populate memcached");
//             log::warn!("Populated Memcached");
//         });
//
//         let manager_clone = manager.clone();
//         let db = tokio::spawn(async move {
//             log::warn!("Populating Mongodb...");
//             manager_clone
//                 .populate_mongodb()
//                 .await
//                 .expect("Failed to populate mongodb");
//             log::warn!("Populated Mongodb");
//         });
//
//         cache.await?;
//         db.await?;
//
//         Ok(manager)
//     }
//
//     async fn populate_memcache(&self) -> Result<(), Box<dyn Error>> {
//         self.memcache.flush()?;
//         let n_hotels = self.hotels as usize;
//         let n_dates = self.dates as usize;
//         let n_conns = self.cache_conns as usize;
//         let mut handles = Vec::new();
//         for i in 0..n_conns {
//             let memcache = self.memcache.clone();
//             let handle = tokio::spawn(async move {
//                 for ave in (i..n_hotels).step_by(n_conns) {
//                     for date in 0..n_dates {
//                         let hotel = Hotel::new(format!("Sheraton_Ave_{}", ave), date as u32, 0, 0);
//                         let hotel_json =
//                             serde_json::to_string(&hotel).expect("Failed to serialize hotel");
//                         memcache
//                             .set(&hotel.key(), hotel_json, 0)
//                             .expect("Failed to set hotel");
//                     }
//                 }
//             });
//             handles.push(handle);
//         }
//         for handle in handles {
//             handle.await?;
//         }
//         Ok(())
//     }
//
//     async fn populate_mongodb(&self) -> Result<(), Box<dyn Error>> {
//         let n_hotels = self.hotels as usize;
//         let n_dates = self.dates as usize;
//         let mut hotels = Vec::new();
//         for ave in 0..n_hotels {
//             for date in 0..n_dates {
//                 let hotel = Hotel::new(format!("Sheraton_Ave_{}", ave), date as u32, 0, 0);
//                 hotels.push(hotel);
//             }
//         }
//         self.collection.insert_many(hotels, None).await?;
//         let index = IndexModel::builder().keys(doc! { "key": 1 }).build();
//         self.collection.create_index(index, None).await?;
//         Ok(())
//     }
//
//     async fn check_availability(&self, hotel: &String, date: u32, _num_rooms: u32) -> bool {
//         let key = format!("{}_{}", hotel, date);
//
//         let _hotel_mc: Option<Hotel> = {
//             if let Ok(Some(hotel_json)) = self.memcache.get::<String>(&key) {
//                 Some(serde_json::from_str(&hotel_json).expect("Failed to deserialize hotel"))
//             } else {
//                 None
//             }
//         };
//
//         let cache_miss = {
//             if _hotel_mc.is_none() {
//                 true
//             } else {
//                 let mut rng = self.rng.lock().expect("Failed to lock rng");
//                 self.uniform_cache_miss.sample(&mut *rng) < self.prob_cache_miss
//             }
//         };
//
//         let _hotel_db = {
//             if !cache_miss {
//                 None
//             } else {
//                 let query = doc! {
//                     "key": &key
//                 };
//                 Some(
//                     self.collection
//                         .find_one(query, None)
//                         .await
//                         .expect("Failed to find hotel")
//                         .expect("Failed to get hotel"),
//                 )
//             }
//         };
//
//         true
//     }
//
//     async fn update_availability(&self, hotel: &String, date: u32, num_rooms: u32) {
//         let key = format!("{}_{}", hotel, date);
//
//         let mut hotel = {
//             if let Ok(Some(hotel_json)) = self.memcache.get::<String>(&key) {
//                 serde_json::from_str(&hotel_json).expect("Failed to deserialize hotel")
//             } else {
//                 let query = doc! {
//                     "key": &key
//                 };
//                 self.collection
//                     .find_one(query, None)
//                     .await
//                     .expect("Failed to find hotel")
//                     .expect("Failed to get hotel")
//             }
//         };
//         hotel.n_reservations += num_rooms;
//
//         let hotel_json = serde_json::to_string(&hotel).expect("Failed to serialize hotel");
//         self.memcache
//             .set(&key, hotel_json, 0)
//             .expect("Failed to set hotel");
//
//         let query = doc! {
//             "key": &key
//         };
//         self.collection
//             .update_one(
//                 query,
//                 doc! { "$set": { "n_reservations": hotel.n_reservations } },
//                 None,
//             )
//             .await
//             .expect("Failed to update hotel");
//     }
// }

pub struct ReservationImpl {
    memc_client: Arc<memcache::Client>,
    mongo_client: Arc<MongoClient>,
    lat_check_avail: Mutex<LatencyTracker>,
    lat_make_reserve: Mutex<LatencyTracker>,
    check_avail_hotel_mc: Arc<AvgTracker>,
    check_avail_hotel_mongo: Arc<AvgTracker>,
    check_avail_reserve: Arc<AvgTracker>,
    check_reserve: Arc<AvgTracker>,
    // manager: HotelManager,
}

impl ReservationImpl {
    pub async fn new(
        hotels: u32,
        dates: u32,
        prob_hotel_avail: u32,
        cache_addr: String,
        cache_conns: u32,
        prob_cache_miss: u32,
        db_addr: String,
    ) -> Result<Self, Box<dyn Error>> {
        let memc_client = memcache::Client::with_pool_size(cache_addr, cache_conns)?;
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
                println!(
                    "CheckAvail: mc {} mongo {} reserve {}; MkReserve: {}",
                    ca_hotel_mc.get(),
                    ca_hotel_mongo.get(),
                    ca_reserve.get(),
                    reserve.get()
                );
            }
        });

        Ok(Self {
            memc_client: Arc::new(memc_client),
            mongo_client: Arc::new(mongo_client),
            lat_check_avail,
            lat_make_reserve,
            check_avail_reserve,
            check_avail_hotel_mc,
            check_avail_hotel_mongo,
            check_reserve,
        })
        // let manager = HotelManager::new(
        //     hotels,
        //     dates,
        //     prob_hotel_avail,
        //     cache_addr,
        //     cache_conns,
        //     prob_cache_miss,
        //     db_addr,
        // )
        // .await?;
        // let reservation = ReservationImpl { manager };
        // Ok(reservation)
    }

    // async fn check_availability(
    //     &self,
    //     request: reservation::ReservationRequest,
    // ) -> reservation::ReservationResponse {
    //     log::info!("request: {:?}", request);
    //     let mut hotels = Vec::new();
    //     // [NOTE] Optional multi-threading.
    //     for hotel in &request.hotels {
    //         for date in request.in_date..request.out_date {
    //             self.manager
    //                 .check_availability(hotel, date, request.num_rooms)
    //                 .await;
    //         }
    //         let hotel_avail = {
    //             let mut rng = self.manager.rng.lock().expect("Failed to lock rng");
    //             self.manager.uniform_hotel_avail.sample(&mut *rng) < self.manager.prob_hotel_avail
    //         };
    //         if hotel_avail {
    //             hotels.push(hotel.clone());
    //         }
    //     }
    //     let response = reservation::ReservationResponse { hotels };
    //     log::info!("response: {:?}", response);
    //     response
    // }
}

#[tonic::async_trait]
impl Reservation for ReservationImpl {
    async fn check_availability(
        &self,
        req: Request<reservation::ReservationRequest>,
    ) -> Result<Response<reservation::ReservationResponse>, Status> {
        use futures::StreamExt;
        // todo!()
        // // let ctx = request.metadata().get_ctx("ctx").unwrap();
        // let request = request.into_inner();
        // let response = self.check_availability(request).await;
        // Ok(Response::new(response))

        let start = Instant::now();

        let req = req.into_inner();

        // Create hotel memory keys and maps
        let mut hotel_mem_keys = Vec::new();
        let mut missing_keys = HashSet::new();
        let mut res_map: HashMap<String, bool> = HashMap::new();

        self.check_avail_hotel_mc.track(req.hotel_id.len());
        for hotel_id in &req.hotel_id {
            let cap_key = format!("{}_cap", hotel_id);
            hotel_mem_keys.push(cap_key.clone());
            missing_keys.insert(cap_key);
            res_map.insert(hotel_id.clone(), true);
        }

        // Get capacity from memcached
        let mut cache_cap = HashMap::new();

        let mc_req: Vec<&str> = hotel_mem_keys.iter().map(|k| k.as_str()).collect();
        let mc_resp = self.memc_client.gets(&mc_req).unwrap();
        for (hotel_id, capacity) in mc_resp {
            if let Ok(cap) = String::from_utf8(capacity)
                .unwrap_or_default()
                .parse::<i32>()
            {
                cache_cap.insert(hotel_id.clone(), cap);
                missing_keys.remove(&hotel_id);
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
                    let mc = self.memc_client.clone();
                    async_executor::spawn(async move {
                        let _ = mc.set(&key, value.as_bytes(), 0);
                    })
                    .detach();
                }
            }
        }

        // Process date ranges and create queries
        let mut query_map = HashMap::new();
        let mut req_commands = Vec::new();

        for hotel_id in &req.hotel_id {
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

        let mc_req: Vec<&str> = req_commands.iter().map(|k| k.as_str()).collect();
        let mc_resp = self.memc_client.gets(&mc_req).unwrap();
        for (cmd, value) in mc_resp {
            let (hotel_id, _, _) = query_map.remove(&cmd).unwrap();
            if let Ok(count) = String::from_utf8(value).unwrap_or_default().parse::<i32>() {
                let cap = cache_cap.get(&format!("{}_cap", hotel_id)).unwrap_or(&0);
                if count + req.room_number > *cap {
                    res_map
                        .entry(hotel_id.to_owned())
                        .and_modify(|e| *e = false);
                }
            }
        }

        // Check reservations in parallel
        let mut tasks = Vec::new();

        self.check_avail_reserve.track(query_map.len());
        for (command, (hotel_id, start_date, end_date)) in query_map {
            let hotel_id = hotel_id.to_owned();
            let start_date = start_date.to_owned();
            let end_date = end_date.to_owned();

            let memc_client = self.memc_client.clone();
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

                    // Update memcached
                    let _ = memc_client.set(&command, count.to_string().as_bytes(), 0);

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
            hotel_id: Vec::new(),
        };
        for (hotel_id, available) in res_map {
            if available {
                resp.hotel_id.push(hotel_id);
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
        // todo!()
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
            hotel_id: Vec::new(),
        };

        let database = self.mongo_client.database("reservation-db");
        let res_collection: Collection<db::Reservation> = database.collection("reservation");
        let num_collection: Collection<db::Number> = database.collection("number");

        let in_date = DateTime::parse_from_rfc3339(&format!("{}T12:00:00+00:00", req.in_date))
            .unwrap()
            .with_timezone(&chrono::Utc);
        let out_date = DateTime::parse_from_rfc3339(&format!("{}T12:00:00+00:00", req.out_date))
            .unwrap()
            .with_timezone(&chrono::Utc);

        let hotel_id = &req.hotel_id[0];
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
            let count = match self.memc_client.get::<String>(&memc_key) {
                Ok(Some(value)) => {
                    // Memcached hit
                    // count = String::from_utf8_lossy(&value).parse::<i32>().unwrap();
                    let count = value.parse::<i32>().unwrap();
                    memc_date_num_map.insert(memc_key, count + req.room_number);
                    count
                }
                Ok(None) => {
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
                Err(e) => panic!("Memcached error: {}", e),
            };

            // Check capacity
            let memc_cap_key = format!("{}_cap", hotel_id);
            let hotel_cap = match self.memc_client.get::<String>(&memc_cap_key) {
                Ok(Some(value)) => {
                    // String::from_utf8_lossy(&value).parse::<i32>().unwrap()
                    value.parse::<i32>().unwrap()
                }
                Ok(None) => {
                    let filter = doc! { "hotelId": hotel_id };
                    let num = num_collection
                        .find_one(filter, None)
                        .await
                        .unwrap()
                        .expect(&format!("should find hotel {}", hotel_id));

                    let cap = num.number;
                    self.memc_client
                        .set(&memc_cap_key, cap.to_string().as_bytes(), 0)
                        .unwrap();
                    cap
                }
                Err(e) => panic!("Memcached error: {}", e),
            };

            if count + req.room_number > hotel_cap {
                return Ok(Response::new(res));
            }
        }
        self.check_reserve.track(iters);

        // Update reservation number cache
        for (key, val) in memc_date_num_map {
            self.memc_client
                .set(&key, val.to_string().as_bytes(), 0)
                .unwrap();
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
        res.hotel_id.push(hotel_id.clone());

        {
            let elapsed = start.elapsed().as_micros();
            self.lat_make_reserve.lock().unwrap().track(elapsed as u64);
        }

        Ok(Response::new(res))
    }
}
