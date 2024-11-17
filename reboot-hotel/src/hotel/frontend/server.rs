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
    pub mod user {
        tonic::include_proto!("user");
    }
}

use std::time::Instant;

use tonic::{transport::Channel, Request, Response, Status};

use hotel::{
    frontend, frontend::frontend_server::Frontend, profile, profile::profile_client::ProfileClient,
    reservation, reservation::reservation_client::ReservationClient, search,
    search::search_client::SearchClient, user, user::user_client::UserClient,
};

pub struct FrontendImpl {
    search_client: SearchClient<Channel>,
    reservation_client: ReservationClient<Channel>,
    profile_client: ProfileClient<Channel>,
    user_client: UserClient<Channel>,
}

impl FrontendImpl {
    pub async fn new(
        search_addr: String,
        reservation_addr: String,
        profile_addr: String,
        user_addr: String,
    ) -> Self {
        let search_client = SearchClient::connect(search_addr)
            .await
            .expect("Failed to connect to search");
        let reservation_client = ReservationClient::connect(reservation_addr)
            .await
            .expect("Failed to connect to reservation");
        let profile_client = ProfileClient::connect(profile_addr)
            .await
            .expect("Failed to connect to search");
        let user_client = UserClient::connect(user_addr)
            .await
            .expect("Failed to connect to user");
        FrontendImpl {
            search_client,
            reservation_client,
            profile_client,
            user_client,
        }
    }
}

#[tonic::async_trait]
impl Frontend for FrontendImpl {
    async fn handle_search(
        &self,
        request: Request<frontend::SearchRequest>,
    ) -> Result<Response<frontend::SearchResponse>, Status> {
        let start = Instant::now();
        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let request = request.into_inner();

        let mut search_client = self.search_client.clone();
        let search_req = search::NearbyRequest {
            lat: request.lat,
            lon: request.lon,
            in_date: request.in_date.clone(),
            out_date: request.out_date.clone(),
        };
        let search_resp = search_client.handle_nearby(search_req).await?;
        let response = search_resp.into_inner();

        let mut reservation_client = self.reservation_client.clone();
        let span_request = reservation::ReservationRequest {
            customer_name: "".into(),
            hotel_id: response.hotel_ids.clone(),
            in_date: request.in_date,
            out_date: request.out_date,
            room_number: 1,
        };
        let span_response = reservation_client.check_availability(span_request).await?;
        let response = span_response.into_inner();

        let mut profile_client = self.profile_client.clone();
        let profile_request = profile::ProfileRequest {
            hotel_ids: response.hotel_id,
            locale: request.locale.unwrap_or("en".to_string()),
        };
        let profile_response = profile_client.get_profiles(profile_request).await?;
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

        let mut response = Response::new(response);
        ctx.set_frontend_elapse(start.elapsed().as_micros() as u64);
        response.metadata_mut().insert_ctx("ctx", &ctx);

        Ok(response)
    }

    async fn handle_reservation(
        &self,
        request: Request<frontend::ReservationRequest>,
    ) -> Result<Response<frontend::ReservationResponse>, Status> {
        let start = Instant::now();
        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let request = request.into_inner();

        let mut user_client = self.user_client.clone();
        let user_request = user::UserRequest {
            username: request.username,
            password: request.password,
        };
        let user_response = user_client.handle_check_user(user_request).await?;
        let response = user_response.into_inner();

        if !response.success {
            return Ok(Response::new(frontend::ReservationResponse {
                hotels: Vec::new(),
            }));
        }

        let mut reservation_client = self.reservation_client.clone();
        let reservation_request = reservation::ReservationRequest {
            customer_name: request.customer,
            hotel_id: request.hotels,
            in_date: request.in_date,
            out_date: request.out_date,
            room_number: 1,
        };
        let reservation_response = reservation_client
            .make_reservation(reservation_request)
            .await?;
        let response = reservation_response.into_inner();

        let response = frontend::ReservationResponse {
            hotels: response.hotel_id,
        };

        let mut response = Response::new(response);
        ctx.set_frontend_elapse(start.elapsed().as_micros() as u64);
        response.metadata_mut().insert_ctx("ctx", &ctx);

        Ok(response)
    }
}
