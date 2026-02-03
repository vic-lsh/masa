pub mod hotel_tonic {
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
    pub mod review {
        tonic::include_proto!("review");
    }
}

use crate::config::HotelConfig;
// use hotel_tonic::review::review_client::ReviewClient;
use std::time::Instant;

use tonic::masa::{MasaRequestExt, MasaResponseExt};
use tonic::{transport::masa_channel::LoadBalancedChannel, Request, Response, Status};

use hotel_tonic::{
    frontend, frontend::frontend_server::Frontend, profile, profile::profile_client::ProfileClient,
    reservation, reservation::reservation_client::ReservationClient, search,
    search::search_client::SearchClient, user, user::user_client::UserClient,
};

use hotel::profile_layer::extract_latency_traces;

pub struct FrontendImpl {
    search_client: SearchClient<LoadBalancedChannel>,
    reservation_client: ReservationClient<LoadBalancedChannel>,
    profile_client: ProfileClient<LoadBalancedChannel>,
    user_client: UserClient<LoadBalancedChannel>,
    // review_client: ReviewClient<LoadBalancedChannel>,
}

impl FrontendImpl {
    pub async fn new(config: HotelConfig) -> Self {
        let channel = LoadBalancedChannel::new(
            config.search.ip.clone(),
            config.search.port,
            config.search.replicas,
        )
        .await;
        let search_client = SearchClient::new(channel);

        let channel = LoadBalancedChannel::new(
            config.reservation.ip.clone(),
            config.reservation.port,
            config.reservation.replicas,
        )
        .await;
        let reservation_client = ReservationClient::new(channel);

        let channel = LoadBalancedChannel::new(
            config.profile.ip.clone(),
            config.profile.port,
            config.profile.replicas,
        )
        .await;
        let profile_client = ProfileClient::new(channel);

        let channel = LoadBalancedChannel::new(
            config.user.ip.clone(),
            config.user.port,
            config.user.replicas,
        )
        .await;
        let user_client = UserClient::new(channel);

        // let channel = LoadBalancedChannel::new(
        //     config.review.ip.clone(),
        //     config.review.port,
        //     config.review.replicas,
        // )
        // .await;
        // let review_client = ReviewClient::new(channel);

        FrontendImpl {
            search_client,
            reservation_client,
            profile_client,
            user_client,
            // review_client,
        }
    }
}

#[tonic::async_trait]
impl Frontend for FrontendImpl {
    async fn handle_ping(
        &self,
        request: Request<frontend::PingRequest>,
    ) -> Result<Response<frontend::PingResponse>, Status> {
        let request = request.into_inner();
        let response = frontend::PingResponse {
            message: request.message,
        };
        let response = Response::new(response);
        Ok(response)
    }

    async fn handle_search(
        &self,
        request: Request<frontend::SearchRequest>,
    ) -> Result<Response<frontend::SearchResponse>, Status> {
        use masa::time_now;

        let start = Instant::now();
        let mut ctx = request.get_masa_context().unwrap();
        let request = request.into_inner();

        let mut child_traces = Vec::new();

        let mut search_client = self.search_client.clone();
        let search_req = search::NearbyRequest {
            lat: request.lat,
            lon: request.lon,
            in_date: request.in_date.clone(),
            out_date: request.out_date.clone(),
        };
        let search_start_time = time_now();
        let search_resp = search_client.handle_nearby(search_req).await?;
        let search_header = search_resp.metadata();
        match search_header.get("X-Latency-Traces") {
            Some(_) => {
                child_traces.push("search".to_string());
                child_traces.push(search_start_time.to_string());
                child_traces.extend(
                    extract_latency_traces(search_header).expect("missing X-Latency-Traces header"),
                );
            }
            None => {}
        }
        let nearby_response = search_resp.into_inner();

        let mut reservation_client = self.reservation_client.clone();
        let span_request = reservation::ReservationRequest {
            customer_name: "".into(),
            hotel_ids: nearby_response.hotel_ids.clone(),
            in_date: request.in_date,
            out_date: request.out_date,
            room_number: 1,
        };

        let reservation_start_time = time_now();
        let span_response = reservation_client.check_availability(span_request).await?;
        let reservation_header = span_response.metadata();
        match reservation_header.get("X-Latency-Traces") {
            Some(_) => {
                child_traces.push("reservation".to_string());
                child_traces.push(reservation_start_time.to_string());
                child_traces.extend(
                    extract_latency_traces(reservation_header)
                        .expect("missing X-Latency-Traces header"),
                );
            }
            None => {}
        }
        let availability_response = span_response.into_inner();

        let mut profile_client = self.profile_client.clone();
        let profile_request = profile::ProfileRequest {
            hotel_ids: availability_response.hotel_ids,
            locale: request.locale.unwrap_or_else(|| "en".to_string()),
        };
        let profile_start_time = time_now();
        let profile_response = profile_client.get_profiles(profile_request).await?;
        let profile_header = profile_response.metadata();
        match profile_header.get("X-Latency-Traces") {
            Some(_) => {
                child_traces.push("profile".to_string());
                child_traces.push(profile_start_time.to_string());
                child_traces.extend(
                    extract_latency_traces(profile_header)
                        .expect("missing X-Latency-Traces header"),
                );
            }
            None => {}
        }
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

        let response = frontend::SearchResponse {
            hotels,
            child_traces,
        };
        let mut response = Response::new(response);
        ctx.set_frontend_elapse(start.elapsed().as_micros() as u64);
        response.set_masa_context(&ctx);

        Ok(response)
    }

    async fn handle_reservation(
        &self,
        request: Request<frontend::ReservationRequest>,
    ) -> Result<Response<frontend::ReservationResponse>, Status> {
        let start = Instant::now();
        let mut ctx = request.get_masa_context().unwrap();
        let request = request.into_inner();

        let mut user_client = self.user_client.clone();
        let user_request = user::UserRequest {
            username: request.username,
            password: request.password,
        };
        let user_response = user_client.check_user(user_request).await?;
        let response = user_response.into_inner();

        if !response.success {
            return Ok(Response::new(frontend::ReservationResponse {
                hotels: Vec::new(),
            }));
        }

        let mut reservation_client = self.reservation_client.clone();
        let reservation_request = reservation::ReservationRequest {
            customer_name: request.customer,
            hotel_ids: request.hotels,
            in_date: request.in_date,
            out_date: request.out_date,
            room_number: 1,
        };
        let reservation_response = reservation_client
            .make_reservation(reservation_request)
            .await?;
        let response = reservation_response.into_inner();

        let response = frontend::ReservationResponse {
            hotels: response.hotel_ids,
        };

        let mut response = Response::new(response);
        ctx.set_frontend_elapse(start.elapsed().as_micros() as u64);
        response.set_masa_context(&ctx);

        Ok(response)
    }

    // async fn handle_review(
    //     &self,
    //     request: Request<frontend::ReviewRequest>,
    // ) -> Result<Response<frontend::ReviewResponse>, Status> {
    //     let request = request.into_inner();
    //     let mut review_client = self.review_client.clone();
    //
    //     let review_req = hotel_tonic::review::ReviewRequest {
    //         hotel_id: request.hotel_id,
    //     };
    //
    //     let review_resp = review_client.get_reviews(review_req).await?;
    //     let review_resp = review_resp.into_inner();
    //
    //     // Map review::ReviewComm to frontend::ReviewComm
    //     let reviews = review_resp
    //         .reviews
    //         .into_iter()
    //         .map(|r| frontend::ReviewComm {
    //             review_id: r.review_id,
    //             hotel_id: r.hotel_id,
    //             name: r.name,
    //             rating: r.rating,
    //             description: r.description,
    //             images: r
    //                 .images
    //                 .into_iter()
    //                 .map(|img| frontend::Image {
    //                     url: img.url,
    //                     r#default: img.default,
    //                 })
    //                 .collect(),
    //         })
    //         .collect();
    //
    //     Ok(Response::new(frontend::ReviewResponse { reviews }))
    // }
    async fn handle_review(
        &self,
        _request: Request<frontend::ReviewRequest>,
    ) -> Result<Response<frontend::ReviewResponse>, Status> {
        Ok(Response::new(frontend::ReviewResponse { reviews: vec![] }))
    }
}
