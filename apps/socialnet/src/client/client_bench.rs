mod gen;

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
use gen::{get_compose_post_request, get_register_user_request};
use masa::{Context, ContextBuilder};
use socialnet::frontend;
use socialnet::frontend::frontend_service_client::FrontendServiceClient;
use tonic::masa::MasaRequestExt;

struct SocialnetClient;

impl Client for SocialnetClient {
    type FrontendClient = FrontendServiceClient<Channel>;

    async fn connect(dst: String) -> Result<Self::FrontendClient, tonic::transport::Error> {
        FrontendServiceClient::connect(dst).await
    }

    async fn ping(client: &mut Self::FrontendClient) -> Result<(), tonic::Status> {
        let ctx = {
            let slo = 1_000_000;
            let start_at = time_now();
            let deadline = start_at + slo;
            let req_id = 0;
            ContextBuilder::new("ping".to_string(), req_id)
                .slo(slo)
                .gateway_entry(start_at)
                .deadline(deadline)
                .build()
        };
        let request = tonic::Request::new(frontend::PingRequest {
            message: "ping".to_string(),
        })
        .with_masa_context(&ctx);
        client.ping(request).await.map(|_| ())
    }
}

enum RequestHandler {
    ComposePostRequest(Handler<ComposePostRequestType, SocialnetClient>),
    RegisterUserRequest(Handler<RegisterUserRequestType, SocialnetClient>),
}

impl HandlerOuter<SocialnetClient> for RequestHandler {
    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self {
        match api {
            "ComposePost" => {
                RequestHandler::ComposePostRequest(Handler::new(api, rps, timeout, slo))
            }
            "RegisterUser" => {
                RequestHandler::RegisterUserRequest(Handler::new(api, rps, timeout, slo))
            }
            _ => panic!("unknown API {}", api),
        }
    }

    async fn send_request(
        &self,
        rng: StdRng,
        client: FrontendServiceClient<Channel>,
        ctx: Context,
        trace: bool,
    ) -> String {
        match self {
            Self::ComposePostRequest(h) => h.send_request(rng, client, ctx, trace).await,
            Self::RegisterUserRequest(h) => h.send_request(rng, client, ctx, trace).await,
        }
    }

    async fn fetch_traces(&mut self, output_path: &Path) {
        match self {
            Self::ComposePostRequest(h) => h.fetch_traces(output_path).await,
            Self::RegisterUserRequest(h) => h.fetch_traces(output_path).await,
        }
    }

    fn api(&self) -> &str {
        match self {
            Self::ComposePostRequest(h) => h.api.as_str(),
            Self::RegisterUserRequest(h) => h.api.as_str(),
        }
    }

    fn slo(&self) -> u64 {
        match self {
            Self::ComposePostRequest(h) => h.slo,
            Self::RegisterUserRequest(h) => h.slo,
        }
    }
}

struct ComposePostRequestType {}

impl RequestType<SocialnetClient> for ComposePostRequestType {
    type ResponseType = socialnet::compose_post::ComposePostResponse;

    fn new(_api: &str) -> Self {
        Self {}
    }

    async fn create_request(
        &self,
        rng: &mut StdRng,
        mut client: FrontendServiceClient<Channel>,
        ctx: &Context,
    ) -> Result<Response<Self::ResponseType>, Status> {
        let r = tonic::Request::new(get_compose_post_request(rng)).with_masa_context(ctx);
        client.compose_post(r).await
    }

    fn response_output_headers(&self) -> Vec<String> {
        Vec::new()
    }

    fn response_to_row(_metadata: &MetadataMap, _r: &Self::ResponseType) -> Vec<String> {
        Vec::new()
    }
}

struct RegisterUserRequestType {}

impl RequestType<SocialnetClient> for RegisterUserRequestType {
    type ResponseType = socialnet::register_user::RegisterUserResponse;

    fn new(_api: &str) -> Self {
        Self {}
    }

    async fn create_request(
        &self,
        rng: &mut StdRng,
        mut client: FrontendServiceClient<Channel>,
        ctx: &Context,
    ) -> Result<Response<Self::ResponseType>, Status> {
        let r = tonic::Request::new(get_register_user_request(rng)).with_masa_context(ctx);
        client.register_user(r).await
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
    load_gen_main::<RequestHandler, SocialnetClient>(args, time_now()).await
}
