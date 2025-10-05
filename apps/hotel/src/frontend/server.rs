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
use tracing::{error, info};
// use hotel_tonic::review::review_client::ReviewClient;
use std::time::Duration;
use std::time::Instant;

use app_utils::retry::retry_until_ok;
use tonic::{transport::{Channel, Endpoint}, Request, Response, Status};
use tower::discover::Change;

use hotel_tonic::{
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
    pub async fn new(config: HotelConfig) -> Self {
        let base_delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(10);

        let (channel, tx) = Channel::balance_channel(32);

        Self::setup_search_service_discovery(&config, tx).await;
        let search_client = SearchClient::new(channel);

        let reservation_addr = format!(
            "http://{}:{}",
            config.reservation.ip.clone(),
            config.reservation.port
        );
        let reservation_client = retry_until_ok(
            || async {
                ReservationClient::connect(reservation_addr.clone())
                    .await
                    .map_err(|e| {
                        error!("Failed to connect to {}", reservation_addr.clone());
                        e
                    })
            },
            base_delay,
            max_delay,
        )
        .await;

        let profile_addr = format!(
            "http://{}:{}",
            config.profile.ip.clone(),
            config.profile.port
        );
        let profile_client = retry_until_ok(
            || async {
                ProfileClient::connect(profile_addr.clone())
                    .await
                    .map_err(|e| {
                        error!("Failed to connect to {}", profile_addr.clone());
                        e
                    })
            },
            base_delay,
            max_delay,
        )
        .await;

        let user_addr = format!("http://{}:{}", config.user.ip.clone(), config.user.port);
        let user_client = retry_until_ok(
            || async {
                UserClient::connect(user_addr.clone()).await.map_err(|e| {
                    error!("Failed to connect to {}", user_addr.clone());
                    e
                })
            },
            base_delay,
            max_delay,
        )
        .await;

        info!("FrontendService launched");

        FrontendImpl {
            search_client,
            reservation_client,
            profile_client,
            user_client,
        }
    }

    /// Sets up service discovery for the search service based on deployment mode
    async fn setup_search_service_discovery(
        config: &HotelConfig,
        tx: tokio::sync::mpsc::Sender<Change<tonic::transport::Uri, Endpoint>>,
    ) {
        if std::env::var("DOCKER_COMPOSE").is_ok() {
            Self::setup_docker_compose_discovery(config, tx).await;
        } else {
            Self::setup_kubernetes_discovery(config, tx).await;
        }
    }

    /// Sets up Docker Compose service discovery
    async fn setup_docker_compose_discovery(
        config: &HotelConfig,
        tx: tokio::sync::mpsc::Sender<Change<tonic::transport::Uri, Endpoint>>,
    ) {
        const HOSTNAME_BASE: &str = "hotel-search-service";
        
        let port = config.search.port;
        let replicas = config.search.replicas;
        
        info!("Docker Compose mode: creating {} replicas for search service", replicas);
        
        tokio::spawn(async move {
            for i in 1..=replicas {
                let endpoint_uri = format!("http://{}-{}:{}", HOSTNAME_BASE, i, port);
                let uri = endpoint_uri.parse().unwrap();
                let endpoint = Endpoint::from_shared(endpoint_uri.clone())
                    .unwrap()
                    .tcp_nodelay(true)
                    .connect_timeout(Duration::from_secs(2));
                
                info!("Adding Docker Compose endpoint: {}", endpoint_uri);
                let _ = tx.send(Change::Insert(uri, endpoint)).await;
            }
        });
    }

    /// Sets up Kubernetes service discovery
    async fn setup_kubernetes_discovery(
        config: &HotelConfig,
        tx: tokio::sync::mpsc::Sender<Change<tonic::transport::Uri, Endpoint>>,
    ) {
        app_utils::balance::spawn_endpointslice_task(
            "hotel".to_string(),
            config.search.ip.clone(),
            None,
            tx,
        )
        .await
        .ok();
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
            hotel_ids: response.hotel_ids.clone(),
            in_date: request.in_date,
            out_date: request.out_date,
            room_number: 1,
        };
        let response = {
            let span_response = reservation_client.check_availability(span_request).await?;
            span_response.into_inner()
        };

        let mut profile_client = self.profile_client.clone();
        let profile_request = profile::ProfileRequest {
            hotel_ids: response.hotel_ids,
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
