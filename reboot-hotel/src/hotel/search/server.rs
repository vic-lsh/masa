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

use ginepro::LoadBalancedChannel;
use tonic::{transport::Channel, Request, Response, Status};

use hotel::{
    geo, geo::geo_client::GeoClient, rate, rate::rate_client::RateClient, search,
    search::search_server::Search,
};

pub struct SearchImpl {
    geo_client: GeoClient<LoadBalancedChannel>,
    rate_client: RateClient<LoadBalancedChannel>,
}

impl SearchImpl {
    pub async fn new(geo_addr: String, rate_addr: String) -> Self {
        let channel = LoadBalancedChannel::builder(("geo-service", 8660))
            .channel()
            .await
            .expect("Failed to connect to user");
        let geo_client = GeoClient::new(channel);

        let channel = LoadBalancedChannel::builder(("rate-service", 8660))
            .channel()
            .await
            .expect("Failed to connect to rate");
        let rate_client = RateClient::new(channel);

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
        let geo_request = geo::NearbyRequest {
            lat: request.lat,
            lon: request.lon,
        };
        let geo_response = geo_client.get_nearby(Request::new(geo_request)).await?;
        let response = geo_response.into_inner();

        let hotel_ids = response.hotel_ids;
        let mut rate_client = self.rate_client.clone();
        let rate_request = rate::RateRequest {
            hotel_ids,
            in_date: request.in_date,
            out_date: request.out_date,
        };
        let rate_response = rate_client.get_rates(Request::new(rate_request)).await?;
        let response = rate_response.into_inner();

        let mut hotel_ids = Vec::new();
        for plan in response.rate_plans {
            hotel_ids.push(plan.hotel_id);
        }
        let response = search::NearbyResponse { hotel_ids };
        Ok(Response::new(response))
    }
}
