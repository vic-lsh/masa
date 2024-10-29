pub mod hotel {
    pub mod rate {
        tonic::include_proto!("rate");
    }
}

use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Uniform};
use std::error::Error;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use mongodb::{bson::doc, Client, Collection, Database, IndexModel};
use serde::{Deserialize, Serialize};
use tonic::{Request, Response, Status};

use hotel::{rate, rate::rate_server::Rate};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotel {
    name: String,
    ave: u32,
    key: String,
    payload: Vec<u8>,
}

#[derive(Clone)]
pub struct HotelManager {
    rng: Arc<Mutex<StdRng>>,
    uniform: Uniform<u64>,
    hotels: u32,
    payload: u32,
    cache_conn: u32,
    prob_cache_miss: u32,
    memcache: memcache::Client,
    _database: Database,
    collection: Collection<Hotel>,
}

impl HotelManager {
    pub async fn new(
        hotels: u32,
        payload: u32,
        cache_addr: String,
        cache_conn: u32,
        prob_cache_miss: u32,
        db_addr: String,
    ) -> Result<Self, Box<dyn Error>> {
        let seed = 998244353;
        let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(seed)));
        let uniform = Uniform::new(0, 100);
        let memcache = memcache::Client::with_pool_size(cache_addr, cache_conn)?;
        let client = Client::with_uri_str(db_addr).await?;
        let database = client.database("sheraton");
        let collection = database.collection::<Hotel>("collection");
        collection.delete_many(doc! {}, None).await?;
        let manager = HotelManager {
            rng,
            uniform,
            hotels,
            payload,
            cache_conn,
            prob_cache_miss,
            memcache,
            _database: database,
            collection,
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
                .populate_mongodb(hotels)
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
        let payload = self.payload as usize;
        let conn = self.cache_conn as usize;
        let mut handles = Vec::new();
        for i in 0..conn {
            let memcache = self.memcache.clone();
            let handle = tokio::spawn(async move {
                for j in (i..n_hotels).step_by(conn) {
                    let hotel = Hotel {
                        key: "rate".to_string(),
                        name: format!("Sheraton Ave {}", j),
                        ave: j as u32,
                        payload: vec![0; payload],
                    };
                    let hotel_json =
                        serde_json::to_string(&hotel).expect("Failed to serialize hotel");
                    memcache
                        .set(hotel.name.as_str(), hotel_json, 0)
                        .expect("Failed to set hotel");
                }
            });
            handles.push(handle);
        }
        for handle in handles {
            handle.await?;
        }
        Ok(())
    }

    async fn populate_mongodb(&self, n_hotels: u32) -> Result<(), Box<dyn Error>> {
        let payload = self.payload as usize;
        let mut hotels = Vec::new();
        for i in 0..n_hotels {
            hotels.push(Hotel {
                key: "rate".to_string(),
                name: format!("Sheraton Ave {}", i),
                ave: i as u32,
                payload: vec![0; payload],
            });
        }
        self.collection.insert_many(hotels, None).await?;
        let index = IndexModel::builder().keys(doc! { "name": 1 }).build();
        self.collection.create_index(index, None).await?;
        Ok(())
    }

    // pub fn fetch_memcache(&self, names: Vec<String>) -> Vec<Hotel> {
    //     let names_ref = names.iter().map(|s| s.as_str()).collect::<Vec<&str>>();
    //     let mut hotels = Vec::new();
    //     if let Ok(hotel_jsons) = self.memcache.gets::<String>(&names_ref) {
    //         for hotel_json in hotel_jsons.values() {
    //             let hotel: Hotel =
    //                 serde_json::from_str(hotel_json).expect("Failed to deserialize hotel");
    //             hotels.push(hotel);
    //         }
    //     }
    //     hotels.sort_by_key(|hotel| hotel.ave);
    //     hotels
    // }

    // pub async fn fetch_mongodb(&self, names: Vec<String>) -> Vec<Hotel> {
    //     let query = doc! {
    //         "name": {
    //             "$in": names
    //         }
    //     };
    //     let mut cursor = self
    //         .collection
    //         .find(query, None)
    //         .await
    //         .expect("Failed to find hotels");
    //     let mut hotels = Vec::new();
    //     while let Some(hotel) = cursor.next().await {
    //         let hotel = hotel.expect("Failed to get hotel");
    //         hotels.push(hotel);
    //     }
    //     // [OPTIONAL] Clear cache
    //     hotels
    // }

    pub async fn fetch_mixture(&self, names: Vec<String>) -> Vec<Hotel> {
        let names_db = {
            let mut rng = self.rng.lock().expect("Failed to lock rng");
            let value = self.uniform.sample(&mut *rng) % 100;
            if value < self.prob_cache_miss as u64 {
                names.clone()
            } else {
                Vec::new()
            }
        };

        let names_ref = names.iter().map(|s| s.as_str()).collect::<Vec<&str>>();
        let mut hotels = Vec::new();
        if let Ok(hotel_jsons) = self.memcache.gets::<String>(&names_ref) {
            for hotel_json in hotel_jsons.values() {
                let hotel: Hotel =
                    serde_json::from_str(hotel_json).expect("Failed to deserialize hotel");
                hotels.push(hotel);
            }
        }
        hotels.sort_by_key(|hotel| hotel.ave);

        if !names_db.is_empty() {
            let query = doc! {
                "name": {
                    "$in": names_db
                }
            };
            let mut cursor = self
                .collection
                .find(query, None)
                .await
                .expect("Failed to find hotels");
            while let Some(hotel) = cursor.next().await {
                let hotel = hotel.expect("Failed to get hotel");
                let hotel_json = serde_json::to_string(&hotel).expect("Failed to serialize hotel");
                self.memcache
                    .set(hotel.name.as_str(), hotel_json, 0)
                    .expect("Failed to set hotel");
            }
        }

        hotels
    }
}

pub struct RateImpl {
    manager: HotelManager,
}

impl RateImpl {
    pub async fn new(
        hotels: u32,
        payload: u32,
        cache_addr: String,
        cache_conn: u32,
        prob_cache_miss: u32,
        db_addr: String,
    ) -> Result<Self, Box<dyn Error>> {
        let manager = HotelManager::new(
            hotels,
            payload,
            cache_addr,
            cache_conn,
            prob_cache_miss,
            db_addr,
        )
        .await?;
        let rate = RateImpl { manager };
        Ok(rate)
    }
}

#[tonic::async_trait]
impl Rate for RateImpl {
    async fn handle_get_rates(
        &self,
        request: Request<rate::RateRequest>,
    ) -> Result<Response<rate::RateResponse>, Status> {
        // let ctx = request.metadata().get_ctx("ctx").unwrap();
        let request = request.into_inner();
        let hotels = self.manager.fetch_mixture(request.hotels).await;
        let mut plans = Vec::new();
        for hotel in hotels {
            plans.push(rate::HotelRate {
                key: "rate".to_string(),
                hotel: hotel.name,
                payload: hotel.payload,
            });
        }
        let response = rate::RateResponse { plans };
        log::info!("response: {:?}", response);
        Ok(Response::new(response))
    }
}
