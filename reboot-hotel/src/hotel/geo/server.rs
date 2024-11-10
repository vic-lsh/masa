pub mod hotel {
    pub mod geo {
        tonic::include_proto!("geo");
    }
}

use std::cmp;
use std::sync::{Arc, Mutex};

use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Uniform};
use tonic::{Request, Response, Status};

use hotel::{geo, geo::geo_server::Geo};

#[derive(Debug, Clone)]
struct Hotel {
    name: String,
    _ave: u32,
}

struct HotelManager {
    rng: Arc<Mutex<StdRng>>,
    uniform: Uniform<u32>,
    hotels: Vec<Hotel>,
}

impl HotelManager {
    fn new(n_hotels: u32, range: u32) -> Self {
        let seed = 998244353;
        let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(seed)));
        let uniform = Uniform::from(0..range);
        let mut hotels = Vec::new();
        for i in 0..n_hotels {
            hotels.push(Hotel {
                name: format!("Sheraton_Ave_{}", i),
                _ave: i as u32,
            });
        }
        HotelManager {
            rng,
            uniform,
            hotels,
        }
    }

    fn fetch(&self, ave: u32) -> Vec<Hotel> {
        let lhs = ave as usize;
        let range = {
            let mut rng = self.rng.lock().expect("Failed to lock rng");
            self.uniform.sample(&mut *rng) as usize
        };
        let rhs = cmp::min(lhs + range, self.hotels.len());
        self.hotels[lhs..rhs].to_vec()
    }
}

pub struct GeoImpl {
    manager: HotelManager,
}

impl GeoImpl {
    pub fn new(hotels: u32, range: u32) -> Self {
        GeoImpl {
            manager: HotelManager::new(hotels, range),
        }
    }
}

#[tonic::async_trait]
impl Geo for GeoImpl {
    async fn handle_nearby(
        &self,
        request: Request<geo::NearbyRequest>,
    ) -> Result<Response<geo::NearbyResponse>, Status> {
        let request = request.into_inner();
        let fetched_hotels = self.manager.fetch(request.ave);
        let mut hotels = Vec::new();
        for hotel in fetched_hotels {
            hotels.push(hotel.name);
        }
        let response = geo::NearbyResponse { hotels };
        log::info!("response: {:?}", response);
        Ok(Response::new(response))
    }
}
