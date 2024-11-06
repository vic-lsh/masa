pub mod hotel {
    pub mod frontend {
        tonic::include_proto!("frontend");
    }
    pub mod search {
        tonic::include_proto!("search");
    }
    pub mod reservation {
        tonic::include_proto!("reservation");
    }
    pub mod profile {
        tonic::include_proto!("profile");
    }
}

use std::time::Instant;

use tonic::{transport::Channel, Request, Response, Status};

use hotel::{
    frontend, frontend::frontend_server::Frontend, profile, profile::profile_client::ProfileClient,
    reservation, reservation::reservation_client::ReservationClient, search,
    search::search_client::SearchClient,
};

pub struct FrontendImpl {
    search_client: SearchClient<Channel>,
    reservation_client: ReservationClient<Channel>,
    profile_client: ProfileClient<Channel>,
}

impl FrontendImpl {
    pub async fn new(search_addr: String, reservation_addr: String, profile_addr: String) -> Self {
        let search_client = SearchClient::connect(search_addr)
            .await
            .expect("Failed to connect to search");
        let reservation_client = ReservationClient::connect(reservation_addr)
            .await
            .expect("Failed to connect to reservation");
        let profile_client = ProfileClient::connect(profile_addr)
            .await
            .expect("Failed to connect to search");
        FrontendImpl {
            search_client,
            reservation_client,
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
        self.handle_search_inner(request).await
    }
}

impl FrontendImpl {
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

        let mut reservation_client = self.reservation_client.clone();
        let span_request = reservation::ReservationRequest {
            customer: request.customer,
            hotels: response.hotels.clone(),
            in_date: request.in_date,
            out_date: request.out_date,
            num_rooms: 0,
        };
        let span_response = reservation_client
            .handle_check_availability(span_request)
            .await?;
        let response = span_response.into_inner();

        let mut profile_client = self.profile_client.clone();
        let profile_request = profile::ProfileRequest {
            hotels: response.hotels,
        };
        let profile_response = profile_client.handle_get_profiles(profile_request).await?;
        let response = profile_response.into_inner();

        let mut hotels = Vec::new();
        for profile in response.profiles {
            hotels.push(frontend::Hotel {
                name: profile.hotel,
            });
        }

        let response = frontend::SearchResponse { hotels };

        let mut response = Response::new(response);
        ctx.set_frontend_elapse(request_start.elapsed().as_micros() as u64);
        response.metadata_mut().insert_ctx("ctx", &ctx);

        Ok(response)
    }
}
