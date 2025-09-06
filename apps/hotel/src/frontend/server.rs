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
use app_utils::channel::LoadBalancedChannel;
// use hotel_tonic::review::review_client::ReviewClient;
use std::time::Instant;

use tonic::{Request, Response, Status};

use hotel_tonic::{
    frontend, frontend::frontend_server::Frontend, profile, profile::profile_client::ProfileClient,
    reservation, reservation::reservation_client::ReservationClient, search,
    search::search_client::SearchClient, user, user::user_client::UserClient,
};

pub struct FrontendImpl {
    search_client: SearchClient<LoadBalancedChannel>,
    reservation_client: ReservationClient<LoadBalancedChannel>,
    profile_client: ProfileClient<LoadBalancedChannel>,
    user_client: UserClient<LoadBalancedChannel>,
    // review_client: ReviewClient<LoadBalancedChannel>,
}

impl FrontendImpl {
    pub async fn new(config: HotelConfig) -> Self {
        let channel =
            LoadBalancedChannel::new(config.search_ip, config.search_port, config.search_replicas)
                .await;
        let search_client = SearchClient::new(channel);

        let channel = LoadBalancedChannel::new(
            config.reservation_ip,
            config.reservation_port,
            config.reservation_replicas,
        )
        .await;
        let reservation_client = ReservationClient::new(channel);

        let channel = LoadBalancedChannel::new(
            config.profile_ip,
            config.profile_port,
            config.profile_replicas,
        )
        .await;
        let profile_client = ProfileClient::new(channel);

        let channel =
            LoadBalancedChannel::new(config.user_ip, config.user_port, config.user_replicas).await;
        let user_client = UserClient::new(channel);

        // let channel =
        //     LoadBalancedChannel::new(config.review_ip, config.review_port, config.review_replicas)
        //         .await;
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


// Helper function that obtain the e2e, io, and queue latencies from the metadata
fn extract_latencies(metadata: &tonic::metadata::MetadataMap) -> (u64, u64, u64, u64) {
    let latency = metadata.get("X-Request-Latency").unwrap().to_str().unwrap();
    let latencies = latency.split(',').collect::<Vec<&str>>();
    let e2e_latency = latencies.get(0).unwrap_or(&"0").parse().unwrap_or(0);
    let compute_latency = latencies.get(1).unwrap_or(&"0").parse().unwrap_or(0);
    let io_latency = latencies.get(2).unwrap_or(&"0").parse().unwrap_or(0);
    let queue_latency = latencies.get(3).unwrap_or(&"0").parse().unwrap_or(0);
    (e2e_latency, compute_latency, io_latency, queue_latency)
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
        let search_start = Instant::now();
        let search_resp = search_client.handle_nearby(search_req).await?;
        let search_e2e = search_start.elapsed().as_micros() as u64;
        let search_header = search_resp.metadata();
        let (_, child_search_compute_lat, _, child_search_queue_lat) = extract_latencies(search_header);
        let search_io = search_e2e.saturating_sub(child_search_compute_lat + child_search_queue_lat);

        let response = search_resp.into_inner();

        let mut reservation_client = self.reservation_client.clone();
        let span_request = reservation::ReservationRequest {
            customer_name: "".into(),
            hotel_ids: response.hotel_ids.clone(),
            in_date: request.in_date,
            out_date: request.out_date,
            room_number: 1,
        };
        let reservation_start = Instant::now();
        let span_response = reservation_client.check_availability(span_request).await?;
        let reservation_e2e = reservation_start.elapsed().as_micros() as u64;
        let (_, child_reserve_compute_lat, _, child_reserve_queue_lat) = extract_latencies(span_response.metadata());
        let reservation_io = reservation_e2e.saturating_sub(child_reserve_compute_lat + child_reserve_queue_lat);
        let response = span_response.into_inner();

        let mut profile_client = self.profile_client.clone();
        let profile_request = profile::ProfileRequest {
            hotel_ids: response.hotel_ids,
            locale: request.locale.unwrap_or("en".to_string()),
        };

        let profile_start = Instant::now();
        let profile_response = profile_client.get_profiles(profile_request).await?;
        let profile_e2e = profile_start.elapsed().as_micros() as u64;
        let (_, child_profile_compute_lat, _, child_profile_queue_lat) = extract_latencies(profile_response.metadata());
        let profile_io = profile_e2e.saturating_sub(child_profile_compute_lat + child_profile_queue_lat);
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
            child_search_e2e_latency: search_e2e,
            child_search_compute_latency: child_search_compute_lat,
            child_search_io_latency: search_io,
            child_search_queue_latency: child_search_queue_lat,

            child_reserve_e2e_latency: reservation_e2e,
            child_reserve_compute_latency: child_reserve_compute_lat,
            child_reserve_io_latency: reservation_io,
            child_reserve_queue_latency: child_reserve_queue_lat,

            child_profile_e2e_latency: profile_e2e,
            child_profile_compute_latency: child_profile_compute_lat,
            child_profile_io_latency: profile_io,
            child_profile_queue_latency: child_profile_queue_lat,
         };

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
        response.metadata_mut().insert_ctx("ctx", &ctx);

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
