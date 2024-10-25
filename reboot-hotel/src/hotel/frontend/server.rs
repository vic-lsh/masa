pub mod hotel {
    pub mod frontend {
        tonic::include_proto!("frontend");
    }
    pub mod search {
        tonic::include_proto!("search");
    }
    pub mod profile {
        tonic::include_proto!("profile");
    }
}

use tonic::{transport::Channel, Request, Response, Status};

use hotel::{
    frontend, frontend::frontend_server::Frontend, profile, profile::profile_client::ProfileClient,
    search, search::search_client::SearchClient,
};

pub struct FrontendImpl {
    search_client: SearchClient<Channel>,
    profile_client: ProfileClient<Channel>,
}

impl FrontendImpl {
    pub async fn new(search_addr: String, profile_addr: String) -> Self {
        let search_client = SearchClient::connect(search_addr)
            .await
            .expect("Failed to connect to search");
        let profile_client = ProfileClient::connect(profile_addr)
            .await
            .expect("Failed to connect to search");
        FrontendImpl {
            search_client,
            profile_client,
        }
    }
}

#[tonic::async_trait]
impl Frontend for FrontendImpl {
    async fn handle_search(
        &self,
        request: Request<frontend::SearchRequest>,
    ) -> Result<Response<frontend::SearchResponse>, Status> {
        let request = request.into_inner();

        let mut search_client = self.search_client.clone();
        let search_req = search::NearbyRequest {
            lat: request.lat,
            lon: request.lon,
            in_date: request.in_date,
            out_date: request.out_date,
        };
        let search_resp = search_client
            .handle_nearby(search_req)
            .await
            .expect("Failed to call search::nearby");
        let response = search_resp.into_inner();

        // [TODO] Reserve.
        // reserve_client.check_availability()
        // ReserveRequest { customer, hotel_ids, in_date, out_date, room_number }

        let mut profile_client = self.profile_client.clone();
        let profile_request = profile::ProfileRequest {
            hotel_ids: response.hotel_ids,
            locale: request.locale.unwrap_or("en".to_string()),
        };
        let profile_response = profile_client
            .get_profiles(profile_request)
            .await
            .expect("Failed to call profile::get_profiles");
        let response = profile_response.into_inner();

        let mut hotels = Vec::new();
        for hotel in response.hotels {
            let addr = hotel.address.unwrap();
            hotels.push(frontend::Hotel {
                name: hotel.name,
                id: hotel.id,
                phone_number: hotel.phone_number,
                lat: addr.lat,
                lon: addr.lon,
            });
        }
        let response = frontend::SearchResponse { hotels };
        Ok(Response::new(response))
    }
}
