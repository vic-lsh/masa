use std::error::Error;

use futures::StreamExt;
use mongodb::{bson::doc, Client, Collection, Database, IndexModel};
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotel {
    name: String,
    ave: u32,
    key: String,
    payload: Vec<u8>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct Manager {
    n_hotels: u32,
    payload: u32,
    cache_conn: u32,
    cache_miss_rate: f32,
    memcache: memcache::Client,
    _database: Database,
    collection: Collection<Hotel>,
}

#[allow(dead_code)]
impl Manager {
    pub async fn new(
        n_hotels: u32,
        payload: u32,
        cache_addr: String,
        cache_conn: u32,
        cache_miss_rate: f32,
        db_addr: String,
    ) -> Result<Self, Box<dyn Error>> {
        let memcache = memcache::Client::with_pool_size(cache_addr, cache_conn)?;
        let client = Client::with_uri_str(db_addr).await?;
        let database = client.database("TungChung");
        let collection = database.collection::<Hotel>("collection");
        collection.delete_many(doc! {}, None).await?;
        let manager = Manager {
            n_hotels,
            payload,
            cache_conn,
            cache_miss_rate,
            memcache,
            _database: database,
            collection,
        };
        let manager_clone = manager.clone();
        let cache = tokio::spawn(async move {
            eprintln!("Populating Memcached...");
            manager_clone
                .populate_memcache()
                .await
                .expect("Failed to populate memcached");
            eprintln!("Populated Memcached");
        });
        let manager_clone = manager.clone();
        let db = tokio::spawn(async move {
            eprintln!("Populating Mongodb...");
            manager_clone
                .populate_mongodb(n_hotels)
                .await
                .expect("Failed to populate mongodb");
            eprintln!("Populated Mongodb");
        });
        cache.await?;
        db.await?;
        Ok(manager)
    }

    async fn populate_memcache(&self) -> Result<(), Box<dyn Error>> {
        self.memcache.flush()?;
        let n_hotels = self.n_hotels as usize;
        let payload = self.payload as usize;
        let conn = self.cache_conn as usize;
        let mut handles = Vec::new();
        for i in 0..conn {
            let memcache = self.memcache.clone();
            let handle = tokio::spawn(async move {
                for j in (i..n_hotels).step_by(conn) {
                    let hotel = Hotel {
                        key: "profile".to_string(),
                        name: format!("Tung Chung Ave {}", j),
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
                key: "profile".to_string(),
                name: format!("Tung Chung Ave {}", i),
                ave: i as u32,
                payload: vec![0; payload],
            });
        }
        self.collection.insert_many(hotels, None).await?;
        let index = IndexModel::builder().keys(doc! { "name": 1 }).build();
        self.collection.create_index(index, None).await?;
        Ok(())
    }

    pub fn fetch_memcache(&self, names: Vec<String>) -> Vec<Hotel> {
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
        hotels
    }

    pub async fn fetch_mongodb(&self, names: Vec<String>) -> Vec<Hotel> {
        let query = doc! {
            "name": {
                "$in": names
            }
        };
        let mut cursor = self
            .collection
            .find(query, None)
            .await
            .expect("Failed to find hotels");
        let mut hotels = Vec::new();
        while let Some(hotel) = cursor.next().await {
            let hotel = hotel.expect("Failed to get hotel");
            hotels.push(hotel);
        }
        // [OPTIONAL] Clear cache
        hotels
    }

    pub async fn fetch_mixture(&self, names: Vec<String>) -> Vec<Hotel> {
        let names_clone = names.clone();
        let mut names_db = Vec::new();
        for name in names_clone {
            let mut rng = rand::thread_rng();
            if rng.gen::<f32>() < self.cache_miss_rate {
                names_db.push(name);
            }
        }

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
