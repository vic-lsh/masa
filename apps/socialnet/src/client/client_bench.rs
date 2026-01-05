mod gen;

use std::collections::HashMap;
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
use socialnet::compose_post;
use socialnet::compose_post::compose_post_service_client::ComposePostServiceClient;
use gen::get_compose_post_request;
use masa::Context;

struct SocialnetClient;

impl Client for SocialnetClient {
    type FrontendClient = ComposePostServiceClient<Channel>;

    async fn connect(dst: String) -> Result<Self::FrontendClient, tonic::transport::Error> {
        ComposePostServiceClient::connect(dst).await
    }

    async fn ping(client: &mut Self::FrontendClient) -> Result<(), tonic::Status> {
        // Option B: Use minimal ComposePost request as ping
        let mut request = tonic::Request::new(compose_post::ComposePostRequest {
            req_id: 0,
            username: "ping_user".to_string(),
            user_id: 0,
            text: "ping".to_string(),
            media_ids: vec![],
            media_types: vec![],
            post_type: 0, // POST = 0
            carrier: HashMap::new(),
        });
        let ctx = {
            let slo = 1_000_000;
            let start_at = time_now();
            let deadline = start_at + slo;
            let req_id = 0;
            Context::new("ping".to_string(), req_id, slo, start_at, deadline)
        };
        request.metadata_mut().insert_ctx("ctx", &ctx);
        client.compose_post(request).await.map(|_| ())
    }
}

enum RequestHandler {
    ComposePostRequest(Handler<ComposePostRequestType, SocialnetClient>),
    // Extensible: Add more request types here in the future
    // HomeTimelineRequest(Handler<HomeTimelineRequestType, SocialnetClient>),
    // UserTimelineRequest(Handler<UserTimelineRequestType, SocialnetClient>),
}

impl HandlerOuter<SocialnetClient> for RequestHandler {
    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self {
        match api {
            "ComposePost" => {
                RequestHandler::ComposePostRequest(Handler::new(api, rps, timeout, slo))
            }
            // Extensible: Add more API handlers here
            // "HomeTimeline" => RequestHandler::HomeTimelineRequest(Handler::new(api, rps, timeout, slo)),
            // "UserTimeline" => RequestHandler::UserTimelineRequest(Handler::new(api, rps, timeout, slo)),
            _ => panic!("unknown API {}", api),
        }
    }

    async fn send_request(
        &self,
        rng: StdRng,
        client: ComposePostServiceClient<Channel>,
        ctx: Context,
        trace: bool,
    ) -> String {
        match self {
            Self::ComposePostRequest(h) => h.send_request(rng, client, ctx, trace).await,
            // Extensible: Add more match arms here
        }
    }

    async fn fetch_traces(&mut self, output_path: &Path) {
        match self {
            Self::ComposePostRequest(h) => h.fetch_traces(output_path).await,
            // Extensible: Add more match arms here
        }
    }

    fn api(&self) -> &str {
        match self {
            Self::ComposePostRequest(h) => h.api.as_str(),
            // Extensible: Add more match arms here
        }
    }

    fn slo(&self) -> u64 {
        match self {
            Self::ComposePostRequest(h) => h.slo,
            // Extensible: Add more match arms here
        }
    }
}

struct ComposePostRequestType {}

impl RequestType<SocialnetClient> for ComposePostRequestType {
    type ResponseType = compose_post::ComposePostResponse;

    fn new(_api: &str) -> Self {
        Self {}
    }

    async fn create_request(
        &self,
        rng: &mut StdRng,
        mut client: ComposePostServiceClient<Channel>,
        ctx: &Context,
    ) -> Result<Response<Self::ResponseType>, Status> {
        let mut r = tonic::Request::new(get_compose_post_request(rng));
        r.metadata_mut().insert_ctx("ctx", &ctx);
        client.compose_post(r).await
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
