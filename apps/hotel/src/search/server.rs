pub mod hotel_tonic {
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

use app_utils::stats::latency::{new_latency_tracker, spawn_p50_logger, SyncLatencyTracker};
use std::time::Duration;
use tonic::{transport::masa_channel::LoadBalancedChannel, Request, Response, Status};

use hotel_tonic::{
    geo, geo::geo_client::GeoClient, rate, rate::rate_client::RateClient, search,
    search::search_server::Search,
};

use crate::config::{GeoConfig, RateConfig};

pub struct SearchImpl {
    geo_client: GeoClient<LoadBalancedChannel>,
    rate_client: RateClient<LoadBalancedChannel>,
    latency_tracker: SyncLatencyTracker,
}

impl SearchImpl {
    pub async fn new(geo: GeoConfig, rate: RateConfig) -> Self {
        let (latency_tracker, latency_consumer) = new_latency_tracker("SearchSvc");
        spawn_p50_logger(latency_consumer, Duration::from_secs(30));

        let channel = LoadBalancedChannel::new(geo.ip.clone(), geo.port, geo.replicas).await;
        let geo_client = GeoClient::new(channel);

        let rate_endpoint = rate.endpoint.clone();
        let channel = LoadBalancedChannel::new(
            rate_endpoint.ip.clone(),
            rate_endpoint.port,
            rate_endpoint.replicas,
        )
        .await;
        let rate_client = RateClient::new(channel);

        SearchImpl {
            geo_client,
            rate_client,
            latency_tracker,
        }
    }
}

#[tonic::async_trait]
impl Search for SearchImpl {
    async fn handle_nearby(
        &self,
        request: Request<search::NearbyRequest>,
    ) -> Result<Response<search::NearbyResponse>, Status> {
        let start = std::time::Instant::now();
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
        self.latency_tracker
            .track(start.elapsed().as_micros().try_into().unwrap());
        Ok(Response::new(response))
    }
}
