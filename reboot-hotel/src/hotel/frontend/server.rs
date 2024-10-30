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
    pub mod rate {
        tonic::include_proto!("rate");
    }
    pub mod geo {
        tonic::include_proto!("geo");
    }
}

use std::time::Instant;
use tonic_masa::PriorityHint;

use tonic::{transport::Channel, Request, Response, Status};

use hotel::{
    frontend, frontend::frontend_server::Frontend, profile, profile::profile_client::ProfileClient,
    search, search::search_client::SearchClient,
};
use hotel::{geo, geo::geo_client::GeoClient, rate, rate::rate_client::RateClient};

pub struct FrontendImpl {
    search_client: SearchClient<Channel>,
    profile_client: ProfileClient<Channel>,
    geo_client: GeoClient<Channel>,
    rate_client: RateClient<Channel>,
}

impl FrontendImpl {
    pub async fn new(
        search_addr: String,
        profile_addr: String,
        geo_addr: String,
        rate_addr: String,
    ) -> Self {
        let geo_client = GeoClient::connect(geo_addr)
            .await
            .expect("Failed to connect to geo");
        let rate_client = RateClient::connect(rate_addr)
            .await
            .expect("Failed to connect to rate");

        let search_client = SearchClient::connect(search_addr)
            .await
            .expect("Failed to connect to search");
        let profile_client = ProfileClient::connect(profile_addr)
            .await
            .expect("Failed to connect to search");
        FrontendImpl {
            geo_client,
            rate_client,
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
        let res = self.handle_search_inner(request).await;
        //if res.is_err() {
        //    async_task::set_task_ddl(Some(PriorityHint::new(u64::MAX)));
        //    async_task::yield_now().await;
        //}
        res
    }
}

impl FrontendImpl {
    async fn handle_search_inner2(
        &self,
        request: Request<frontend::SearchRequest>,
    ) -> Result<Response<frontend::SearchResponse>, Status> {
        let request_start = Instant::now();
        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let request = request.into_inner();

        // let mut geo_client = self.geo_client.clone();
        // let geo_request = geo::NearbyRequest { ave: request.ave };
        // let geo_response = geo_client.handle_nearby(Request::new(geo_request)).await?;
        // let response = geo_response.into_inner();

        // let hotels = response.hotels;
        let hotels = (0..4).map(|i| format!("Sheraton Ave {}", i)).collect();
        let mut rate_client = self.rate_client.clone();
        let rate_request = rate::RateRequest { hotels };
        let rate_response = rate_client
            .handle_get_rates(Request::new(rate_request))
            .await;
        let rate_response = match rate_response {
            Ok(r) => {
                //log::warn!("resp {:?}", r);
                r
            }
            Err(e) => {
                //log::warn!("err {:?}", e);
                return Err(e);
            }
        };
        let response = rate_response.into_inner();

        let mut hotels = Vec::new();
        for plan in response.plans {
            hotels.push(hotel::frontend::Hotel { name: plan.hotel });
        }
        let response = frontend::SearchResponse { hotels };

        let mut response = Response::new(response);
        ctx.set_frontend_elapse(request_start.elapsed().as_micros() as u64);
        response.metadata_mut().insert_ctx("ctx", &ctx);

        Ok(response)
    }

    async fn handle_search_inner(
        &self,
        request: Request<frontend::SearchRequest>,
    ) -> Result<Response<frontend::SearchResponse>, Status> {
        let request_start = Instant::now();
        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let request = request.into_inner();

        let mut search_client = self.search_client.clone();
        let span_request = search::NearbyRequest { ave: request.ave };
        let span_response = search_client.handle_nearby(span_request).await?;
        let response = span_response.into_inner();

        // [DEBUG] Twice.
        // let span_request = search::NearbyRequest { ave: request.ave };
        // let span_response = search_client.handle_nearby(span_request).await?;
        // let response = span_response.into_inner();

        // [TODO] Reserve.
        // reserve_client.check_availability()
        // ReserveRequest { customer, hotel_ids, in_date, out_date, room_number }

        // let mut profile_client = self.profile_client.clone();
        // let profile_request = profile::ProfileRequest {
        //     hotels: response.hotels,
        // };
        // let profile_response = profile_client.handle_get_profiles(profile_request).await?;
        // let response = profile_response.into_inner();

        // let mut hotels = Vec::new();
        // for profile in response.profiles {
        //     hotels.push(frontend::Hotel {
        //         name: profile.hotel,
        //     });
        // }

        // let response = frontend::SearchResponse { hotels };
        let response = frontend::SearchResponse { hotels: Vec::new() };

        let mut response = Response::new(response);
        ctx.set_frontend_elapse(request_start.elapsed().as_micros() as u64);
        response.metadata_mut().insert_ctx("ctx", &ctx);

        Ok(response)
    }
}
