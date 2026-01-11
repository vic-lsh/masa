#[path = "../config.rs"]
pub mod config;
pub mod frontend {
    tonic::include_proto!("frontend");
}
mod gen;

use app_utils::load_gen::Handler;
use gen::{get_reservation_request, get_search_request};

use std::path::Path;
use std::time::Duration;

use app_utils::load_gen::load_gen_main;
use app_utils::load_gen::Client;
use http::Uri;
use rand::rngs::StdRng;
use structopt::StructOpt;
use tokio::net::lookup_host;
use tonic::metadata::MetadataMap;
use tonic::transport::{Channel, Endpoint};

use app_utils::{
    load_gen::{HandlerOuter, LoadGenArgs, RequestType},
    timing::time_now,
};
use frontend::frontend_client::FrontendClient;
use hotel::profile_layer::extract_latency_traces;
use masa::Context;
use tonic::Response;
use tonic::Status;

struct HotelClient;

impl Client for HotelClient {
    type FrontendClient = FrontendClient<Channel>;

    async fn connect(dst: String) -> Result<Self::FrontendClient, tonic::transport::Error> {
        let replicas = std::env::var("FRONTEND_REPLICAS")
            .ok()
            .and_then(|val| val.parse::<u8>().ok())
            .unwrap_or(1);
        let (normalized, host, port) = match parse_frontend_addr(&dst) {
            Some(parsed) => parsed,
            None => {
                return FrontendClient::connect(dst).await;
            }
        };

        if replicas > 1 {
            print!("Resolving host {} for {} replicas...\n", host, replicas);
            let addrs = lookup_host((host.as_str(), port)).await;
            if let Ok(addrs) = addrs {
                let endpoints: Result<Vec<_>, _> = addrs
                    .map(|addr| Endpoint::from_shared(format!("http://{}", addr)))
                    .collect();
                let endpoints = endpoints?;
                if endpoints.len() > 1 {
                    let channel = Channel::balance_list(endpoints.into_iter());
                    return Ok(FrontendClient::new(channel));
                }
            }
        }
        FrontendClient::connect(normalized).await
    }

    async fn ping(client: &mut Self::FrontendClient) -> Result<(), tonic::Status> {
        let mut request = tonic::Request::new(frontend::PingRequest {
            message: "ping".to_string(),
        });
        let ctx = {
            let slo = 1_000_000;
            let start_at = time_now();
            let deadline = start_at + slo;
            let req_id = 0;
            Context::new("ping".to_string(), req_id, slo, start_at, deadline)
        };
        request.metadata_mut().insert_ctx("ctx", &ctx);
        client.handle_ping(request).await.map(|_| ())
    }
}

enum RequestHandler {
    ReservationRequest(Handler<ReservationRequest, HotelClient>),
    SearchRequest(Handler<SearchRequest, HotelClient>),
}

impl HandlerOuter<HotelClient> for RequestHandler {
    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self {
        match api {
            "Reservation" => {
                RequestHandler::ReservationRequest(Handler::new(api, rps, timeout, slo))
            }
            "Search" => RequestHandler::SearchRequest(Handler::new(api, rps, timeout, slo)),
            _ => panic!("unknown API {}", api),
        }
    }

    async fn send_request(
        &self,
        rng: StdRng,
        client: FrontendClient<Channel>,
        ctx: Context,
        trace: bool,
    ) -> String {
        match self {
            Self::ReservationRequest(h) => h.send_request(rng, client, ctx, trace).await,
            Self::SearchRequest(h) => h.send_request(rng, client, ctx, trace).await,
        }
    }

    async fn fetch_traces(&mut self, output_path: &Path) {
        match self {
            Self::ReservationRequest(h) => h.fetch_traces(output_path).await,
            Self::SearchRequest(h) => h.fetch_traces(output_path).await,
        }
    }

    fn api(&self) -> &str {
        match self {
            Self::ReservationRequest(h) => h.api.as_str(),
            Self::SearchRequest(h) => h.api.as_str(),
        }
    }

    fn slo(&self) -> u64 {
        match self {
            Self::ReservationRequest(h) => h.slo,
            Self::SearchRequest(h) => h.slo,
        }
    }
}

struct ReservationRequest {}

impl RequestType<HotelClient> for ReservationRequest {
    type ResponseType = frontend::ReservationResponse;

    fn new(_api: &str) -> Self {
        Self {}
    }

    async fn create_request(
        &self,
        rng: &mut StdRng,
        mut client: FrontendClient<Channel>,
        ctx: &Context,
    ) -> Result<Response<Self::ResponseType>, Status> {
        let mut r = tonic::Request::new(get_reservation_request(rng));
        r.metadata_mut().insert_ctx("ctx", &ctx);
        client.handle_reservation(r).await
    }

    fn response_output_headers(&self) -> Vec<String> {
        Vec::new()
    }

    fn response_to_row(_metadata: &MetadataMap, _r: &Self::ResponseType) -> Vec<String> {
        Vec::new()
    }
}

struct SearchRequest {}

// impl SearchRequest {
//     const HEADERS: [&'static str; 12] = [
//         "child_search_e2e_latency",
//         "child_search_compute_latency",
//         "child_search_io_latency",
//         "child_search_queue_latency",
//         "child_reserve_e2e_latency",
//         "child_reserve_compute_latency",
//         "child_reserve_io_latency",
//         "child_reserve_queue_latency",
//         "child_profile_e2e_latency",
//         "child_profile_compute_latency",
//         "child_profile_io_latency",
//         "child_profile_queue_latency",
//     ];
// }

impl RequestType<HotelClient> for SearchRequest {
    type ResponseType = frontend::SearchResponse;

    fn new(_api: &str) -> Self {
        Self {}
    }

    async fn create_request(
        &self,
        rng: &mut StdRng,
        mut client: FrontendClient<Channel>,
        ctx: &Context,
    ) -> Result<Response<Self::ResponseType>, Status> {
        let mut r = tonic::Request::new(get_search_request(rng));
        r.metadata_mut().insert_ctx("ctx", &ctx);
        client.handle_search(r).await
    }

    fn response_output_headers(&self) -> Vec<String> {
        Vec::new()
    }

    fn response_to_row(metadata: &MetadataMap, _r: &Self::ResponseType) -> Vec<String> {
        match metadata.get("X-Latency-Traces") {
            None => Vec::new(),
            Some(_) => {
                let mut traces = extract_latency_traces(metadata).unwrap_or_default();
                traces.extend(_r.child_traces.iter().cloned());
                traces
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = LoadGenArgs::from_args();
    load_gen_main::<RequestHandler, HotelClient>(args, time_now()).await
}

fn parse_frontend_addr(addr: &str) -> Option<(String, String, u16)> {
    let normalized = if addr.contains("://") {
        addr.to_string()
    } else {
        format!("http://{}", addr)
    };
    let uri: Uri = normalized.parse().ok()?;
    let host = uri.host()?.to_string();
    let port = uri.port_u16()?;
    Some((normalized, host, port))
}
