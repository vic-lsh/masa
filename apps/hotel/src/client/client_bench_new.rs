#[path = "../config.rs"]
pub mod config;
pub mod frontend {
    tonic::include_proto!("frontend");
}
use std::error::Error;
use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::io::Write;
use std::iter::zip;
use std::path::Path;
use std::sync::Arc;

use app_utils::load_gen::load_gen_main;
use rand::{thread_rng, Rng};
use structopt::StructOpt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinSet;
use tokio::time::error::Elapsed;
use tonic::metadata::MetadataMap;
use tonic::transport::Channel;

use app_utils::{
    load_gen::{Counters, GenConfig, LoadGenArgs},
    logging::init_logging_file,
    timing::time_now,
};
use frontend::frontend_client::FrontendClient;
use masa::Context;
use tonic::Response;
use tonic::Status;

enum RequestHandler {
    ReservationRequest(ReservationRequest),
    SearchRequest(ReservationRequest),
}

impl Handler for RequestHandler {
    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self {
        if api == "a" {
            RequestHandler::ReservationRequest(ReservationRequest::new(api, rps, timeout, slo))
        } else if api.starts_with(PRESAMPLED_PREFIX) {
            RequestHandler::PresampledRequest(PresampledRequest::new(api, rps, timeout, slo))
        } else {
            panic!("unknown API {}", api)
        }
    }

    async fn send_request(
        &self,
        client: FrontendClient<Channel>,
        ctx: Context,
        trace: bool,
    ) -> String {
        match self {
            Self::ReservationRequest(s) => s.send_request(client, ctx, trace).await,
            Self::PresampledRequest(s) => s.send_request(client, ctx, trace).await,
        }
    }

    async fn fetch_traces(&mut self, output_path: &Path) {
        match self {
            Self::ReservationRequest(s) => s.fetch_traces(output_path).await,
            Self::PresampledRequest(s) => s.fetch_traces(output_path).await,
        }
    }

    fn api(&self) -> &str {
        match self {
            Self::ReservationRequest(s) => s.api(),
            Self::PresampledRequest(s) => s.api(),
        }
    }

    fn slo(&self) -> u64 {
        match self {
            Self::ReservationRequest(s) => s.slo(),
            Self::PresampledRequest(s) => s.slo(),
        }
    }
}

struct ReservationRequest {
    trace_tx: Option<UnboundedSender<RequestStats<Self>>>,
    trace_rx: UnboundedReceiver<RequestStats<Self>>,
    rps: u64,
    timeout: Duration,
    slo: u64,
}

impl ReservationRequest {
    const HEADERS: [&'static str; 7] = [
        "frontend_latency",
        "child1_queueing_latency",
        "child1_sleep_latency",
        "child1_handler_latency",
        "child2_queueing_latency",
        "child2_handler_latency",
        "child2_reply_latency",
    ];

    const API: &'static str = "a";
}

impl RequestType for ReservationRequest {
    type R = frontend::AResponse;

    fn new(_api: &str, rps: u64, timeout: Duration, slo: u64) -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            trace_tx: Some(tx),
            trace_rx: rx,
            rps,
            timeout,
            slo,
        }
    }

    async fn create_request(
        &self,
        mut client: FrontendClient<Channel>,
        ctx: &Context,
    ) -> Result<Response<Self::R>, Status> {
        let mut r = tonic::Request::new(frontend::ARequest {});
        r.metadata_mut().insert_ctx("ctx", &ctx);
        client.handle_a(r).await
    }

    fn response_output_headers(&self) -> Vec<String> {
        Self::HEADERS.iter().map(|s| s.to_string()).collect()
    }

    fn response_to_row(_metadata: &MetadataMap, r: &Self::R) -> Vec<String> {
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

    fn rps(&self) -> u64 {
        self.rps
    }

    fn api(&self) -> &str {
        Self::API
    }

    fn slo(&self) -> u64 {
        self.slo
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    fn trace_tx(&self) -> Option<&UnboundedSender<RequestStats<Self>>> {
        self.trace_tx.as_ref()
    }

    fn drop_trace_tx(&mut self) {
        self.trace_tx.take();
    }

    fn trace_rx(&mut self) -> &mut UnboundedReceiver<RequestStats<Self>> {
        &mut self.trace_rx
    }
}

struct PresampledRequest {
    trace_tx: Option<UnboundedSender<RequestStats<Self>>>,
    trace_rx: UnboundedReceiver<RequestStats<Self>>,
    rps: u64,
    timeout: Duration,
    slo: u64,
    api: String,
}

impl PresampledRequest {}

impl RequestType for PresampledRequest {
    type R = frontend::PresampledResponse;

    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            trace_tx: Some(tx),
            trace_rx: rx,
            rps,
            timeout,
            slo,
            api: api.to_string(),
        }
    }

    async fn create_request(
        &self,
        mut client: FrontendClient<Channel>,
        ctx: &Context,
    ) -> Result<Response<Self::R>, Status> {
        let mut r = tonic::Request::new(frontend::PresampledRequest {
            request_type: self.api[PRESAMPLED_PREFIX.len()..].to_string(),
        });
        r.metadata_mut().insert_ctx("ctx", &ctx);
        client.handle_presampled(r).await
    }

    fn response_output_headers(&self) -> Vec<String> {
        Vec::new()
    }

    fn response_to_row(_metadata: &MetadataMap, _r: &Self::R) -> Vec<String> {
        Vec::new()
    }

    fn rps(&self) -> u64 {
        self.rps
    }

    fn api(&self) -> &str {
        &self.api
    }

    fn slo(&self) -> u64 {
        self.slo
    }

    fn trace_tx(&self) -> Option<&UnboundedSender<RequestStats<Self>>> {
        self.trace_tx.as_ref()
    }

    fn drop_trace_tx(&mut self) {
        self.trace_tx.take();
    }

    fn trace_rx(&mut self) -> &mut UnboundedReceiver<RequestStats<Self>> {
        &mut self.trace_rx
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = LoadGenArgs::from_args();
    load_gen_main(args, thread_rng()).await?
}
