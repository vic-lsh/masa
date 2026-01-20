pub mod frontend {
    tonic::include_proto!("frontend");
}
use std::path::Path;

use rand::rngs::StdRng;
use structopt::StructOpt;
use tokio::time::Duration;
use tonic::metadata::MetadataMap;
use tonic::transport::Channel;
use tonic::Response;
use tonic::Status;

use app_utils::{
    load_gen::{load_gen_main, Client, Handler, HandlerOuter, LoadGenArgs, RequestType},
    timing::time_now,
};
use frontend::frontend_client::FrontendClient;
use masa::{Context, ContextBuilder};

struct SyntheticClient;

impl Client for SyntheticClient {
    type FrontendClient = FrontendClient<Channel>;

    async fn connect(dst: String) -> Result<Self::FrontendClient, tonic::transport::Error> {
        FrontendClient::connect(dst).await
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
            ContextBuilder::new("ping".to_string(), req_id)
                .slo(slo)
                .start_at(start_at)
                .deadline(deadline)
                .build()
        };
        request.metadata_mut().insert_ctx("ctx", &ctx);
        client.handle_ping(request).await.map(|_| ())
    }
}

enum RequestHandler {
    ARequest(Handler<ARequest, SyntheticClient>),
    BRequest(Handler<BRequest, SyntheticClient>),
}

impl HandlerOuter<SyntheticClient> for RequestHandler {
    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self {
        if api == "a" {
            RequestHandler::ARequest(Handler::new(api, rps, timeout, slo))
        } else if api == "b" {
            RequestHandler::BRequest(Handler::new(api, rps, timeout, slo))
        } else {
            panic!("unknown API {}", api)
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
            Self::ARequest(h) => h.send_request(rng, client, ctx, trace).await,
            Self::BRequest(h) => h.send_request(rng, client, ctx, trace).await,
        }
    }

    async fn fetch_traces(&mut self, output_path: &Path) {
        match self {
            Self::ARequest(h) => h.fetch_traces(output_path).await,
            Self::BRequest(h) => h.fetch_traces(output_path).await,
        }
    }

    fn api(&self) -> &str {
        match self {
            Self::ARequest(h) => h.api.as_str(),
            Self::BRequest(h) => h.api.as_str(),
        }
    }

    fn slo(&self) -> u64 {
        match self {
            Self::ARequest(h) => h.slo,
            Self::BRequest(h) => h.slo,
        }
    }
}

struct ARequest {}

impl ARequest {
    const HEADERS: [&'static str; 7] = [
        "frontend_latency",
        "child1_queueing_latency",
        "child1_sleep_latency",
        "child1_handler_latency",
        "child2_queueing_latency",
        "child2_handler_latency",
        "child2_reply_latency",
    ];
}

impl RequestType<SyntheticClient> for ARequest {
    type ResponseType = frontend::AResponse;

    fn new(_api: &str) -> Self {
        Self {}
    }

    async fn create_request(
        &self,
        _rng: &mut StdRng,
        mut client: FrontendClient<Channel>,
        ctx: &Context,
    ) -> Result<Response<Self::ResponseType>, Status> {
        let mut r = tonic::Request::new(frontend::ARequest {});
        r.metadata_mut().insert_ctx("ctx", &ctx);
        client.handle_a(r).await
    }

    fn response_output_headers(&self) -> Vec<String> {
        Self::HEADERS.iter().map(|s| s.to_string()).collect()
    }

    fn response_to_row(_metadata: &MetadataMap, r: &Self::ResponseType) -> Vec<String> {
        vec![
            r.handler_latency.to_string(),
            r.child1_queueing_latency.to_string(),
            r.child1_sleep_latency.to_string(),
            r.child1_handler_latency.to_string(),
            r.child2_queueing_latency.to_string(),
            r.child2_handler_latency.to_string(),
            r.child2_reply_latency.to_string(),
        ]
    }
}

struct BRequest {}

impl RequestType<SyntheticClient> for BRequest {
    type ResponseType = frontend::BResponse;

    fn new(_api: &str) -> Self {
        Self {}
    }

    async fn create_request(
        &self,
        _rng: &mut StdRng,
        mut client: FrontendClient<Channel>,
        ctx: &Context,
    ) -> Result<Response<Self::ResponseType>, Status> {
        let mut r = tonic::Request::new(frontend::BRequest {});
        r.metadata_mut().insert_ctx("ctx", &ctx);
        client.handle_b(r).await
    }

    fn response_output_headers(&self) -> Vec<String> {
        Vec::new()
    }

    fn response_to_row(_metadata: &MetadataMap, _r: &Self::ResponseType) -> Vec<String> {
        Vec::new()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = LoadGenArgs::from_args();
    load_gen_main::<RequestHandler, SyntheticClient>(args, time_now()).await
}
