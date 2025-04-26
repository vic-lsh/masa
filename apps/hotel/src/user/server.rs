pub mod hotel {
    pub mod user {
        tonic::include_proto!("user");
    }
}

use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Uniform};
use std::error::Error;
use std::sync::{Arc, Mutex};

use mongodb::{bson::doc, Client, Collection, Database, IndexModel};
use serde::{Deserialize, Serialize};
use tonic::{Request, Response, Status};

use hotel::{user, user::user_server::User};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    username: String,
    password: String,
}

#[derive(Clone)]
pub struct HotelManager {
    rng: Arc<Mutex<StdRng>>,
    uniform_check_user: Uniform<u32>,
    users: u32,
    _database: Database,
    collection: Collection<Account>,
    prob_check_user: u32,
}

impl HotelManager {
    pub async fn new(
        users: u32,
        db_addr: String,
        prob_check_user: u32,
    ) -> Result<Self, Box<dyn Error>> {
        let seed = 998244353;
        let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(seed)));
        let uniform_check_user = Uniform::new(0, 100);
        let client = Client::with_uri_str(db_addr).await?;
        let database = client.database("sheraton");
        let collection = database.collection::<Account>("collection");
        collection.delete_many(doc! {}, None).await?;
        let manager = HotelManager {
            rng,
            uniform_check_user,
            users,
            _database: database,
            collection,
            prob_check_user,
        };
        let manager_clone = manager.clone();
        let db = tokio::spawn(async move {
            log::warn!("Populating Mongodb...");
            manager_clone
                .populate_mongodb()
                .await
                .expect("Failed to populate mongodb");
            log::warn!("Populated Mongodb");
        });
        db.await?;
        Ok(manager)
    }

    async fn populate_mongodb(&self) -> Result<(), Box<dyn Error>> {
        let mut users = Vec::new();
        for i in 0..self.users {
            users.push(Account {
                username: format!("Username_{}", i),
                password: format!("Password_{}", i),
            });
        }
        self.collection.insert_many(users, None).await?;
        let index = IndexModel::builder().keys(doc! { "username": 1 }).build();
        self.collection.create_index(index, None).await?;
        Ok(())
    }
}

pub struct UserImpl {
    manager: HotelManager,
}

impl UserImpl {
    pub async fn new(
        users: u32,
        db_addr: String,
        prob_check_user: u32,
    ) -> Result<Self, Box<dyn Error>> {
        let manager = HotelManager::new(users, db_addr, prob_check_user).await?;
        let user = UserImpl { manager };
        Ok(user)
    }
}

#[tonic::async_trait]
impl User for UserImpl {
    async fn check_user(
        &self,
        request: Request<user::UserRequest>,
    ) -> Result<Response<user::UserResponse>, Status> {
        let request = request.into_inner();
        log::info!("request: {:?}", request);

        let success = {
            let query = doc! { "username": request.username };
            let account = self.manager.collection.find_one(query, None).await;
            if account.is_err() {
                false
            } else {
                let check_user = {
                    let mut rng = self.manager.rng.lock().unwrap();
                    self.manager.uniform_check_user.sample(&mut *rng) < self.manager.prob_check_user
                };
                if check_user {
                    true
                } else {
                    false
                }
            }
        };

        let response = user::UserResponse { success };
        log::info!("response: {:?}", response);
        Ok(Response::new(response))
    }
}
