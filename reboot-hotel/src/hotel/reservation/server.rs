pub mod hotel {
    pub mod reservation {
        tonic::include_proto!("reservation");
    }
}

use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Uniform};
use std::error::Error;
use std::sync::{Arc, Mutex};

use mongodb::{bson::doc, Client, Collection, Database, IndexModel};
use serde::{Deserialize, Serialize};
use tonic::{Request, Response, Status};

use hotel::{reservation, reservation::reservation_server::Reservation};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotel {
    name: String,
    date: u32,
    key: String,
    n_reservations: u32,
    n_capacity: u32,
}

impl Hotel {
    pub fn new(name: String, date: u32, n_reservations: u32, n_capacity: u32) -> Self {
        let key = format!("{}_{}", name, date);
        Hotel {
            name,
            date,
            key,
            n_reservations,
            n_capacity,
        }
    }

    pub fn key(&self) -> &String {
        &self.key
    }
}

#[derive(Clone)]
pub struct HotelManager {
    rng: Arc<Mutex<StdRng>>,
    uniform_hotel_avail: Uniform<u64>,
    uniform_cache_miss: Uniform<u64>,
    hotels: u32,
    prob_hotel_avail: u32,
    dates: u32,
    cache_conns: u32,
    prob_cache_miss: u32,
    memcache: memcache::Client,
    _database: Database,
    collection: Collection<Hotel>,
}

impl HotelManager {
    pub async fn new(
        hotels: u32,
        dates: u32,
        prob_hotel_avail: u32,
        cache_addr: String,
        cache_conns: u32,
        prob_cache_miss: u32,
        db_addr: String,
    ) -> Result<Self, Box<dyn Error>> {
        let manager = {
            let seed = 998244353;
            let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(seed)));
            let uniform_hotel_avail = Uniform::new(0, 100);
            let uniform_cache_miss = Uniform::new(0, 100);
            let memcache = memcache::Client::with_pool_size(cache_addr, cache_conns)?;
            let client = Client::with_uri_str(db_addr).await?;
            let database = client.database("sheraton");
            let collection = database.collection::<Hotel>("collection");
            collection.delete_many(doc! {}, None).await?;
            HotelManager {
                rng,
                uniform_hotel_avail,
                uniform_cache_miss,
                hotels,
                prob_hotel_avail,
                dates,
                cache_conns,
                prob_cache_miss,
                memcache,
                _database: database,
                collection,
            }
        };

        let manager_clone = manager.clone();
        let cache = tokio::spawn(async move {
            log::info!("Populating Memcached...");
            manager_clone
                .populate_memcache()
                .await
                .expect("Failed to populate memcached");
            log::info!("Populated Memcached");
        });

        let manager_clone = manager.clone();
        let db = tokio::spawn(async move {
            log::info!("Populating Mongodb...");
            manager_clone
                .populate_mongodb()
                .await
                .expect("Failed to populate mongodb");
            log::info!("Populated Mongodb");
        });

        cache.await?;
        db.await?;

        Ok(manager)
    }

    async fn populate_memcache(&self) -> Result<(), Box<dyn Error>> {
        self.memcache.flush()?;
        let n_hotels = self.hotels as usize;
        let n_dates = self.dates as usize;
        let n_conns = self.cache_conns as usize;
        let mut handles = Vec::new();
        for i in 0..n_conns {
            let memcache = self.memcache.clone();
            let handle = tokio::spawn(async move {
                for ave in (i..n_hotels).step_by(n_conns) {
                    for date in 0..n_dates {
                        let hotel = Hotel::new(format!("Sheraton Ave {}", ave), date as u32, 0, 0);
                        let hotel_json =
                            serde_json::to_string(&hotel).expect("Failed to serialize hotel");
                        memcache
                            .set(&hotel.key(), hotel_json, 0)
                            .expect("Failed to set hotel");
                    }
                }
            });
            handles.push(handle);
        }
        for handle in handles {
            handle.await?;
        }
        Ok(())
    }

    async fn populate_mongodb(&self) -> Result<(), Box<dyn Error>> {
        let n_hotels = self.hotels as usize;
        let n_dates = self.dates as usize;
        let mut hotels = Vec::new();
        for ave in 0..n_hotels {
            for date in 0..n_dates {
                let hotel = Hotel::new(format!("Sheraton Ave {}", ave), date as u32, 0, 0);
                hotels.push(hotel);
            }
        }
        self.collection.insert_many(hotels, None).await?;
        let index = IndexModel::builder().keys(doc! { "key": 1 }).build();
        self.collection.create_index(index, None).await?;
        Ok(())
    }

    async fn check_availability(&self, hotel: &String, date: u32, _num_rooms: u32) -> bool {
        let key = format!("{}_{}", hotel, date);

        let hotel_mc: Option<Hotel> = {
            if let Ok(Some(hotel_json)) = self.memcache.get::<String>(&key) {
                Some(serde_json::from_str(&hotel_json).expect("Failed to deserialize hotel"))
            } else {
                None
            }
        };

        let cache_miss = {
            if hotel_mc.is_none() {
                true
            } else {
                let mut rng = self.rng.lock().expect("Failed to lock rng");
                self.uniform_cache_miss.sample(&mut *rng) < self.prob_cache_miss as u64
            }
        };

        let hotel_db = {
            if !cache_miss {
                None
            } else {
                let query = doc! {
                    "key": &key
                };
                Some(
                    self.collection
                        .find_one(query, None)
                        .await
                        .expect("Failed to find hotel")
                        .expect("Failed to get hotel"),
                )
            }
        };

        if let Some(hotel_mc) = hotel_mc {
            if let Some(hotel_db) = hotel_db {
                assert!(
                    hotel_mc.n_reservations == hotel_db.n_reservations,
                    "Reservations should match"
                );
            }
        }

        true
    }

    async fn update_availability(&self, hotel: &String, date: u32, num_rooms: u32) {
        let key = format!("{}_{}", hotel, date);

        let mut hotel = {
            if let Ok(Some(hotel_json)) = self.memcache.get::<String>(&key) {
                serde_json::from_str(&hotel_json).expect("Failed to deserialize hotel")
            } else {
                let query = doc! {
                    "key": &key
                };
                self.collection
                    .find_one(query, None)
                    .await
                    .expect("Failed to find hotel")
                    .expect("Failed to get hotel")
            }
        };
        hotel.n_reservations += num_rooms;

        let hotel_json = serde_json::to_string(&hotel).expect("Failed to serialize hotel");
        self.memcache
            .set(&key, hotel_json, 0)
            .expect("Failed to set hotel");

        let query = doc! {
            "key": &key
        };
        self.collection
            .update_one(
                query,
                doc! { "$set": { "n_reservations": hotel.n_reservations } },
                None,
            )
            .await
            .expect("Failed to update hotel");
    }
}

pub struct ReservationImpl {
    manager: HotelManager,
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
        let manager = HotelManager::new(
            hotels,
            dates,
            prob_hotel_avail,
            cache_addr,
            cache_conns,
            prob_cache_miss,
            db_addr,
        )
        .await?;
        let reservation = ReservationImpl { manager };
        Ok(reservation)
    }

    async fn check_availability(
        &self,
        request: reservation::ReservationRequest,
    ) -> reservation::ReservationResponse {
        let mut hotels = Vec::new();
        // [NOTE] Optional multi-threading.
        for hotel in &request.hotels {
            for date in request.in_date..request.out_date {
                self.manager
                    .check_availability(hotel, date, request.num_rooms)
                    .await;
            }
            let hotel_avail = {
                let mut rng = self.manager.rng.lock().expect("Failed to lock rng");
                self.manager.uniform_hotel_avail.sample(&mut *rng)
                    < self.manager.prob_hotel_avail as u64
            };
            if hotel_avail {
                hotels.push(hotel.clone());
            }
        }
        let response = reservation::ReservationResponse { hotels };
        log::info!("requeset: {:?}, response: {:?}", request, response);
        response
    }
}

#[tonic::async_trait]
impl Reservation for ReservationImpl {
    async fn handle_check_availability(
        &self,
        request: Request<reservation::ReservationRequest>,
    ) -> Result<Response<reservation::ReservationResponse>, Status> {
        // let ctx = request.metadata().get_ctx("ctx").unwrap();
        let request = request.into_inner();
        let response = self.check_availability(request).await;
        Ok(Response::new(response))
    }

    async fn handle_make_reservation(
        &self,
        request: Request<reservation::ReservationRequest>,
    ) -> Result<Response<reservation::ReservationResponse>, Status> {
        let request = request.into_inner();
        let response = self.check_availability(request.clone()).await;
        // [NOTE] Optional multi-threading.
        for hotel in &response.hotels {
            for date in request.in_date..request.out_date {
                self.manager
                    .update_availability(hotel, date, request.num_rooms)
                    .await;
            }
        }
        Ok(Response::new(response))
    }
}
