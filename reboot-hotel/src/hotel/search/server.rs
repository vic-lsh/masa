pub mod hotel {
    pub mod search {
        tonic::include_proto!("search");
    }
    pub mod geo {
        tonic::include_proto!("geo");
    }
    pub mod rate {
        tonic::include_proto!("rate");
    }
}

use tonic::{transport::Channel, Request, Response, Status};

use hotel::{
    geo, geo::geo_client::GeoClient, rate, rate::rate_client::RateClient, search,
    search::search_server::Search,
};

pub struct SearchImpl {
    geo_client: GeoClient<Channel>,
    rate_client: RateClient<Channel>,
}

impl SearchImpl {
    pub async fn new(geo_addr: String, rate_addr: String) -> Self {
        let geo_client = GeoClient::connect(geo_addr)
            .await
            .expect("Failed to connect to geo");
        let rate_client = RateClient::connect(rate_addr)
            .await
            .expect("Failed to connect to rate");
        SearchImpl {
            geo_client,
            rate_client,
        }
    }
}

#[tonic::async_trait]
impl Search for SearchImpl {
    async fn handle_nearby(
        &self,
        request: Request<search::NearbyRequest>,
    ) -> Result<Response<search::NearbyResponse>, Status> {
        let request = request.into_inner();

        let mut geo_client = self.geo_client.clone();
        let geo_request = geo::NearbyRequest { ave: request.ave };
        let geo_response = geo_client.handle_nearby(Request::new(geo_request)).await;
        if geo_response.is_err() {
            return Err(geo_response.unwrap_err());
        }
        let geo_response = geo_response.expect("Failed to call geo::nearby");
        let response = geo_response.into_inner();

        let hotels = response.hotels;
        let mut rate_client = self.rate_client.clone();
        let rate_request = rate::RateRequest { hotels };
        let rate_response = rate_client
            .handle_get_rates(Request::new(rate_request))
            .await;
        if rate_response.is_err() {
            return Err(rate_response.unwrap_err());
        }
        let rate_response = rate_response.expect("Failed to call rate::get_rates");
        let response = rate_response.into_inner();

        let mut hotels = Vec::new();
        for plan in response.plans {
            hotels.push(plan.hotel);
        }
        let response = search::NearbyResponse { hotels };
        Ok(Response::new(response))
    }
}
