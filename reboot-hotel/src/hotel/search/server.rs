use masa::{
    geo, geo::geo_client::GeoClient, rate, rate::rate_client::RateClient, search,
    search::search_server::Search,
};
use tonic::{transport::Channel, Request, Response, Status};

pub mod masa {
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
        let span_request = geo::NearbyRequest { ave: request.ave };
        let span_response = geo_client
            .handle_nearby(Request::new(span_request))
            .await
            .expect("Failed to call geo::nearby");
        let response = span_response.into_inner();

        let hotels = response.hotels;
        let mut rate_client = self.rate_client.clone();
        let span_request = rate::RateRequest { hotels };
        let span_response = rate_client
            .handle_get_rates(Request::new(span_request))
            .await
            .expect("Failed to call rate::get_rates");
        let response = span_response.into_inner();

        let mut hotels = Vec::new();
        for plan in response.plans {
            hotels.push(plan.hotel);
        }
        let response = search::NearbyResponse { hotels };
        Ok(Response::new(response))
    }
}
