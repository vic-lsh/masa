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

use rand::{thread_rng, Rng};
use structopt::StructOpt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinSet;
use tokio::time::error::Elapsed;
use tokio::time::{timeout, Duration, Instant};
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

const COUNTER_KEYS: [&'static str; 6] = [
    // total number of (sent) requests
    "all",
    // number of (completed) requests satisfying SLO
    "good",
    // number of early returns
    "err_svc_er",
    // number of deadline misses without timing out
    "err_cl_miss",
    // number of timeouts
    "err_cl_to",
    // total number of unexpected errors
    "unexpected",
];

const PRESAMPLED_PREFIX: &'static str = "presampled_";

trait RequestType
where
    Self: Sized,
{
    type R;

    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self;

    async fn get_request(
        &self,
        client: FrontendClient<Channel>,
        ctx: &Context,
    ) -> Result<Response<Self::R>, Status>;

    async fn send_request(
        &self,
        client: FrontendClient<Channel>,
        ctx: Context,
        trace: bool,
    ) -> String {
        let latency;
        let response = {
            let send_at = time_now();
            let r = timeout(self.timeout(), self.get_request(client, &ctx)).await;
            let recv_at = time_now();
            latency = recv_at - send_at;
            r
        };

        let (response, error) = map_response(response, latency <= self.slo());

        let stats = RequestStats::new(ctx, latency, error.clone(), response);

        if trace {
            self.trace_tx().send(stats).unwrap();
        }

        error
    }

    fn response_output_headers(&self) -> Vec<String>;

    fn response_to_row(metadata: &MetadataMap, response: &Self::R) -> Vec<String>;

    fn rps(&self) -> u64;

    fn slo(&self) -> u64;

    fn api(&self) -> &str;

    fn timeout(&self) -> Duration;

    fn trace_tx(&self) -> &UnboundedSender<RequestStats<Self>>;

    fn trace_rx(&mut self) -> &mut UnboundedReceiver<RequestStats<Self>>;

    fn header_row(&self) -> String {
        let generic = RequestStats::<Self>::HEADERS.join(",");
        let specific = self.response_output_headers().join(",");
        format!("{},{}", generic, specific)
    }

    async fn fetch_traces(&mut self, output_path: &Path) {
        let mut file =
            File::create(output_path.join(format!("r{}_{}.csv", self.rps(), self.api()))).unwrap();
        writeln!(file, "{}", self.header_row()).unwrap();
        while let Some(stats) = self.trace_rx().recv().await {
            writeln!(file, "{}", stats.to_row()).unwrap();
        }
        log::info!("All traces fetched");
    }
}

// async fn fetch_traces<T>(
//     output_path: &Path,
//     rps: u64,
//     api: &str,
//     trace_receiver: &mut Receiver<RequestStats<T>>,
// ) where
//     T: RequestType,
// {
// let mut file = File::create(output_path.join(format!("r{}_{}.csv", rps, api))).unwrap();
// writeln!(file, "{}", RequestStats::<T>::header_row()).unwrap();
// while let Ok(stats) = trace_receiver.recv() {
//     writeln!(file, "{}", stats.to_row()).unwrap();
// }
// log::info!("All traces fetched");
// }

enum RequestHandler {
    ARequest(ARequest),
    PresampledRequest(PresampledRequest),
}

impl RequestHandler {
    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self {
        if api == "a" {
            RequestHandler::ARequest(ARequest::new(api, rps, timeout, slo))
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
            Self::ARequest(s) => s.send_request(client, ctx, trace).await,
            Self::PresampledRequest(s) => s.send_request(client, ctx, trace).await,
        }
    }

    async fn fetch_traces(&mut self, output_path: &Path) {
        match self {
            Self::ARequest(s) => s.fetch_traces(output_path).await,
            Self::PresampledRequest(s) => s.fetch_traces(output_path).await,
        }
    }

    fn api(&self) -> &str {
        match self {
            Self::ARequest(s) => s.api(),
            Self::PresampledRequest(s) => s.api(),
        }
    }

    fn slo(&self) -> u64 {
        match self {
            Self::ARequest(s) => s.slo(),
            Self::PresampledRequest(s) => s.slo(),
        }
    }
}

struct ARequest {
    trace_tx: UnboundedSender<RequestStats<Self>>,
    trace_rx: UnboundedReceiver<RequestStats<Self>>,
    rps: u64,
    timeout: Duration,
    slo: u64,
}

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

    const API: &'static str = "a";
}

impl RequestType for ARequest {
    type R = frontend::AResponse;

    fn new(_api: &str, rps: u64, timeout: Duration, slo: u64) -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            trace_tx: tx,
            trace_rx: rx,
            rps,
            timeout,
            slo,
        }
    }

    async fn get_request(
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

    fn trace_tx(&self) -> &UnboundedSender<RequestStats<Self>> {
        &self.trace_tx
    }

    fn trace_rx(&mut self) -> &mut UnboundedReceiver<RequestStats<Self>> {
        &mut self.trace_rx
    }
}

struct PresampledRequest {
    trace_tx: UnboundedSender<RequestStats<Self>>,
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
            trace_tx: tx,
            trace_rx: rx,
            rps,
            timeout,
            slo,
            api: api.to_string(),
        }
    }

    async fn get_request(
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

    fn trace_tx(&self) -> &UnboundedSender<RequestStats<Self>> {
        &self.trace_tx
    }

    fn trace_rx(&mut self) -> &mut UnboundedReceiver<RequestStats<Self>> {
        &mut self.trace_rx
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }
}

#[derive(Debug)]
struct RequestStats<T>
where
    T: RequestType,
{
    ctx: Context,
    latency: u64,
    // frontend_latency: u64,
    // child1_queueing_latency: u64,
    // child1_sleep_latency: u64,
    // child1_handler_latency: u64,
    // child2_queueing_latency: u64,
    // child2_handler_latency: u64,
    // child2_reply_latency: u64,
    error: String,
    response: Option<(MetadataMap, T::R)>,
}

impl<T> RequestStats<T>
where
    T: RequestType,
{
    const HEADERS: [&'static str; 7] = [
        "api",
        "request_id",
        "slo",
        "start_at",
        "deadline",
        "latency",
        "error",
    ];

    fn new(
        ctx: Context,
        latency: u64,
        error: String,
        response: Option<(MetadataMap, T::R)>,
    ) -> Self {
        Self {
            ctx,
            latency,
            error,
            response,
        }
    }

    fn to_row(&self) -> String {
        let generic = format!(
            "{},{},{},{},{},{},{}",
            self.ctx.api(),
            self.ctx.request_id(),
            self.ctx.slo(),
            self.ctx.start_at(),
            self.ctx.deadline(),
            self.latency,
            self.error
        );

        let specific = match &self.response {
            Some((m, r)) => T::response_to_row(&m, &r).join(","),
            None => "".to_string(),
        };

        format!("{},{}", generic, specific)
    }
}

struct LoadGenerator {
    gen_cfg: GenConfig,
    rps: u64,
    client: FrontendClient<Channel>,
    api_handlers: Vec<Arc<RequestHandler>>,
}

impl LoadGenerator {
    pub fn new(
        gen_cfg: GenConfig,
        rps: u64,
        client: FrontendClient<Channel>,
        api_handlers: Vec<Arc<RequestHandler>>,
    ) -> Self {
        Self {
            gen_cfg,
            rps,
            client,
            api_handlers,
        }
    }

    async fn run(&mut self, output_path: &Path) -> Result<(), Box<dyn Error>> {
        let init_at = Instant::now();
        let warm_at = init_at + Duration::from_secs_f64(self.gen_cfg.warmup_secs as f64 / 2.0);
        let trace_at = init_at + Duration::from_secs(self.gen_cfg.warmup_secs);
        let pause_at =
            init_at + Duration::from_secs(self.gen_cfg.warmup_secs + self.gen_cfg.duration_secs);

        let counters = Arc::new(Counters::new(&COUNTER_KEYS));

        let h = tokio::task::spawn(stats_logger(Arc::clone(&counters), pause_at));

        self.generate_load(counters, init_at, trace_at, warm_at, pause_at)
            .await;

        let _ = h.await;

        log::info!("Load generated, writing trace");

        let handlers = self
            .api_handlers
            .drain(..)
            .map(|h| Arc::into_inner(h).unwrap());

        // output traces
        for mut handler in handlers {
            handler.fetch_traces(output_path).await
        }

        Ok(())
    }

    async fn generate_load(
        &mut self,
        counters: Arc<Counters>,
        init_at: Instant,
        trace_at: Instant,
        warm_at: Instant,
        pause_at: Instant,
    ) {
        let mut counter_request_id = 0;
        let mut elapse = 0f64;

        let mut set = JoinSet::new();

        while Instant::now() < pause_at {
            // XXX: tokio's sleep has millisecond granularity, so for small `elapse` this may be
            // inaccurate
            let start_at = init_at + Duration::from_secs_f64(elapse);
            tokio::time::sleep_until(start_at).await;

            let value = {
                if Instant::now() < warm_at {
                    0.01
                } else {
                    1f64 / self.rps as f64
                }
            };
            elapse += value;

            let i = thread_rng().gen_range(0..self.api_handlers.len());
            let handler = Arc::clone(&self.api_handlers[i]);

            let ctx = {
                let request_id = counter_request_id;
                counter_request_id += 1;

                let start_at = time_now();
                let deadline = start_at + handler.slo();

                Context::new(
                    handler.api().to_string(),
                    0,
                    request_id,
                    handler.slo(),
                    0,
                    start_at,
                    deadline,
                )
            };

            let client = self.client.clone();
            let ctrs = Arc::clone(&counters);

            set.spawn(async move {
                ctrs.increment("all");

                let trace = Instant::now() > trace_at;

                let error = handler.send_request(client, ctx, trace).await;

                if trace {
                    // increment the right counters
                    match error.as_str() {
                        "/None" => {
                            ctrs.increment("good");
                        }
                        "/ClientMiss" => {
                            ctrs.increment("err_cl_miss");
                        }
                        "/EarlyReturn" => {
                            ctrs.increment("err_svc_er");
                        }
                        "/ClientTimeout" => {
                            ctrs.increment("err_cl_to");
                        }
                        e => {
                            ctrs.increment("unexpected");
                            log::error!("unexpected request error '{}'", e);
                        }
                    };
                }
            });
        }

        // wait for all outgoing requests to complete
        while let Some(_) = set.join_next().await {}
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = LoadGenArgs::from_args();

    let output_path = Path::new(&args.output_path);
    if !output_path.exists() {
        fs::create_dir_all(output_path).unwrap();
    }

    let log_file = if args.save_logs {
        Some(format!("{}/loadgen.log", args.output_path))
    } else {
        None
    };
    init_logging_file(log_file);

    let gen_cfg: GenConfig = {
        let file = File::open(args.gen_config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };
    assert!(gen_cfg.gap == "const" || gen_cfg.gap == "exp");

    for rps in &gen_cfg.rps_values {
        log::info!("Running rps: {}...", rps);

        let mut api_handlers = Vec::new();
        for (api, (timeout_ms, slo)) in zip(&gen_cfg.apis, zip(&gen_cfg.timeouts_ms, &gen_cfg.slos))
        {
            api_handlers.push(Arc::new(RequestHandler::new(
                api,
                *rps,
                Duration::from_millis(*timeout_ms),
                *slo,
            )));
        }

        let mut load_gen = {
            let client = {
                let mut client = FrontendClient::connect(gen_cfg.addr.clone()).await?;
                let mut request = tonic::Request::new(frontend::PingRequest {
                    message: "ping".to_string(),
                });
                let ctx = {
                    let test_id = 0;
                    let request_id = 0;
                    let slo = 1_000_000;
                    let request_class = 0;
                    let start_at = time_now();
                    let deadline = start_at + slo;
                    Context::new(
                        "Ping".to_string(),
                        test_id,
                        request_id,
                        slo,
                        request_class,
                        start_at,
                        deadline,
                    )
                };
                request.metadata_mut().insert_ctx("ctx", &ctx);
                client.handle_ping(request).await?;
                client
            };

            let load_gen = LoadGenerator::new(gen_cfg.clone(), *rps, client, api_handlers);
            load_gen
        };

        load_gen.run(output_path).await.unwrap();
    }

    log::info!("Load generator done");
    Ok(())
}

async fn stats_logger(counters: Arc<Counters>, pause_at: Instant) {
    let mut secs = 0;
    let mut prev = Counters::new(&COUNTER_KEYS);
    while Instant::now() < pause_at {
        tokio::time::sleep(Duration::from_secs(1)).await;
        secs += 1;

        let delta = |k| counters.get(k) - prev.get(k);

        log::warn!(
            "secs: {}, rps: {}, goodput: {}, early returns: {}, deadline misses: {}, timeouts: {}",
            secs,
            delta("all"),
            delta("good"),
            delta("err_svc_er"),
            delta("err_cl_miss"),
            delta("err_cl_to"),
        );
        log::warn!(
            "total early returns: {}, total deadline misses: {}, total timeouts: {}, total unexpected errors: {}",
            counters.get("err_svc_er"),
            counters.get("err_cl_miss"),
            counters.get("err_cl_to"),
            counters.get("unexpected"),
        );
        // clone the Counters struct itself as opposed to creating another reference
        prev = (*counters).clone();
    }
}

fn map_response<T>(
    response: Result<Result<Response<T>, Status>, Elapsed>,
    met_slo: bool,
) -> (Option<(MetadataMap, T)>, String) {
    let error = match &response {
        Ok(response) => {
            if let Err(ref status) = response {
                status.message().to_string()
            } else if !met_slo {
                "/ClientMiss".to_string()
            } else {
                "/None".to_string()
            }
        }
        Err(_) => "/ClientTimeout".to_string(),
    };
    let response = match response {
        Ok(r) => r
            .map(|r| {
                let t = r.into_parts();
                (t.0, t.1)
            })
            .ok(),
        Err(_) => None,
    };
    (response, error)
}
