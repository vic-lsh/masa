pub mod hotel {
    pub mod geo {
        tonic::include_proto!("geo");
    }
}

use tonic::{Request, Response, Status};

use hotel::{geo, geo::geo_server::Geo};

#[derive(Debug, Clone)]
struct Hotel {
    name: String,
    _ave: u32,
}

struct HotelManager {
    hotels: Vec<Hotel>,
    range: u32,
}

impl HotelManager {
    fn new(n_hotels: u32, range: u32) -> Self {
        let mut hotels = Vec::new();
        for i in 0..n_hotels {
            hotels.push(Hotel {
                name: format!("Sheraton Ave {}", i),
                _ave: i as u32,
            });
        }
        HotelManager { hotels, range }
    }

    fn fetch(&self, ave: u32) -> Vec<Hotel> {
        let ave = ave as usize;
        let range = self.range as usize;
        let end = (ave + range).min(self.hotels.len());
        self.hotels[ave..end].to_vec()
    }
}

pub struct GeoImpl {
    // manager: HotelManager,
}

impl GeoImpl {
    pub fn new() -> Self {
        GeoImpl {
            // manager: HotelManager::new(10_000, 5),
        }
    }
}

#[tonic::async_trait]
impl Geo for GeoImpl {
    async fn handle_nearby(
        &self,
        request: Request<geo::NearbyRequest>,
    ) -> Result<Response<geo::NearbyResponse>, Status> {
        // let request = request.into_inner();
        // let fetched_hotels = self.manager.fetch(request.ave);
        let mut hotels = Vec::new();
        // for hotel in fetched_hotels {
        //     hotels.push(hotel.name);
        // }
        let response = geo::NearbyResponse { hotels };
        println!("{:?}", response);
        Ok(Response::new(response))
    }
}
