use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::fs::File;
use std::fs::{self, File};
use std::io::BufReader;
use std::io::BufReader;
use std::io::Write;
use std::iter::zip;
use std::iter::zip;
use std::path::Path;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::{timeout, Duration, Instant};

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
use masa::Context;
use tonic::Response;
use tonic::Status;

use masa::Context;
use serde::{Deserialize, Serialize};
use structopt::StructOpt;
use tonic::metadata::MetadataMap;
use tonic::transport::Channel;
use tonic::Response;
use tonic::Status;

use crate::logging::init_logging_file;
use crate::timing::time_now;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenConfig {
    #[serde(rename = "Repeats")]
    pub repeats: u64,
    #[serde(rename = "Apis")]
    pub apis: Vec<String>,
    #[serde(rename = "Slos")]
    pub slos: Vec<u64>,
    #[serde(rename = "Timeouts_ms")]
    pub timeouts_ms: Vec<u64>,
    #[serde(rename = "Rps")]
    pub rps_values: Vec<u64>,
    #[serde(rename = "Gap")]
    pub gap: String,
    #[serde(rename = "WarmupSecs")]
    pub warmup_secs: u64,
    #[serde(rename = "DurationSecs")]
    pub duration_secs: u64,
    #[serde(rename = "Concurrency")]
    pub concurrency: usize,
    #[serde(rename = "Addr")]
    pub addr: String,
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct LoadGenArgs {
    #[structopt(long, required = true)]
    pub gen_config: PathBuf,
    #[structopt(long, required = true)]
    pub output_path: String,
    #[structopt(long)]
    pub save_logs: bool,
}

const DEFAULT_COUNTER_KEYS: [&'static str; 6] = [
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

pub struct Counters {
    counters_map: HashMap<String, AtomicUsize>,
}

impl Clone for Counters {
    fn clone(&self) -> Self {
        let mut cloned = HashMap::new();
        for k in self.counters_map.keys() {
            cloned.insert(k.clone(), AtomicUsize::new(self.get(&k)));
        }
        Self {
            counters_map: cloned,
        }
    }
}

impl Counters {
    pub fn new(keys: &[&str]) -> Self {
        let mut map = HashMap::new();
        for k in keys {
            map.insert(k.to_string(), AtomicUsize::new(0));
        }
        Self { counters_map: map }
    }

    pub fn get(&self, k: &str) -> usize {
        self.counters_map.get(k).unwrap().load(Ordering::SeqCst)
    }

    pub fn increment(&self, k: &str) {
        self.counters_map
            .get(k)
            .expect(&format!("key '{}' not present", k))
            .fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct RequestStats<T>
where
    T: RequestType,
{
    ctx: Context,
    latency: u64,
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

trait RequestType<R, C>
where
    Self: Sized,
{
    type R;

    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self;

    async fn create_request(
        &self,
        client: C::FrontendClient,
        ctx: &Context,
    ) -> Result<Response<Self::R>, Status>;

    async fn send_request(
        &self,
        client: C::FrontendClient,
        ctx: Context,
        trace: bool,
    ) -> String {
        let latency;
        let response = {
            let send_at = time_now();
            let r = timeout(self.timeout(), self.create_request(client, &ctx)).await;
            let recv_at = time_now();
            latency = recv_at - send_at;
            r
        };

        let (response, error) = map_response(response, latency <= self.slo());

        let stats = RequestStats::new(ctx, latency, error.clone(), response);

        if trace {
            self.trace_tx().unwrap().send(stats).unwrap();
        }

        error
    }

    fn response_output_headers(&self) -> Vec<String>;

    fn response_to_row(metadata: &MetadataMap, response: &Self::R) -> Vec<String>;

    fn rps(&self) -> u64;

    fn slo(&self) -> u64;

    fn api(&self) -> &str;

    fn timeout(&self) -> Duration;

    fn trace_tx(&self) -> Option<&UnboundedSender<RequestStats<Self>>>;

    fn drop_trace_tx(&mut self);

    fn trace_rx(&mut self) -> &mut UnboundedReceiver<RequestStats<Self>>;

    fn header_row(&self) -> String {
        let generic = RequestStats::<Self>::HEADERS.join(",");
        let specific = self.response_output_headers().join(",");
        format!("{},{}", generic, specific)
    }

    async fn fetch_traces(&mut self, output_path: &Path) {
        // must drop so that channel closes
        self.drop_trace_tx();
        let mut file =
            File::create(output_path.join(format!("r{}_{}.csv", self.rps(), self.api()))).unwrap();
        writeln!(file, "{}", self.header_row()).unwrap();
        while let Some(stats) = self.trace_rx().recv().await {
            writeln!(file, "{}", stats.to_row()).unwrap();
        }
        log::info!("All traces fetched");
    }
}

trait Client 
{
    type FrontendClient;

    pub async fn connect(dst: String) -> Result<Self, tonic::transport::Error>;

    pub async fn ping(client: Self::FrontendClient) -> Result<(), tonic::Status>;

}

trait Handler<C> where C: Client {
    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self; 

    async fn send_request(
        &self,
        client: C::FrontendClient,
        ctx: Context,
        trace: bool,
    );

    async fn fetch_traces(&mut self, output_path: &Path);

    fn api(&self) -> &str; 

    fn slo(&self) -> u64;
}

pub struct LoadGenerator<R, H, C>
where
        R: Rng,
    H: Handler,
    C: Client,
{
    rng: R,
    gen_cfg: GenConfig,
    rps: u64,
    client: C::FrontendClient,
    api_handlers: Vec<Arc<H>>,
}

impl<R, H, C> LoadGenerator<R, H, C>
where
    R: Rng,
    H: Handler,
    C: Client,
{
    pub fn new(
        rng: Rng, 
        gen_cfg: GenConfig,
        rps: u64,
        client: C::FrontendClient,
        api_handlers: Vec<Arc<H>>,
    ) -> Self {
        Self {
            rng,
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

        let counter_keys = &DEFAULT_COUNTER_KEYS;
        let counters = Arc::new(Counters::new(counter_keys));

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

            let i = self.rng.gen_range(0..self.api_handlers.len());
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

                // TODO: count error per handler
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

pub async fn load_gen_main<R, H, C>(args: LoadGenArgs, rng: R) -> Result<(), Box<dyn std::error::Error>>
where
    R: Rng
    H: Handler,
    C: Client,
{
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

        assert_eq!(gen_cfg.apis.len(), gen_cfg.timeouts_ms.len());
        assert_eq!(gen_cfg.apis.len(), gen_cfg.slos.len());
        let mut api_handlers = Vec::new();
        for (api, (timeout_ms, slo)) in zip(&gen_cfg.apis, zip(&gen_cfg.timeouts_ms, &gen_cfg.slos))
        {
            api_handlers.push(Arc::new(H::new(
                api,
                *rps,
                Duration::from_millis(*timeout_ms),
                *slo,
            )));
        }

        let mut load_gen = {
            let client = {
                let mut client = C::connect(gen_cfg.addr.clone()).await?;
                C::ping(client).await?;
                client
            };

            let load_gen = LoadGenerator::new::<R, H, C>(rng, gen_cfg.clone(), *rps, client, api_handlers);
            load_gen
        };

        load_gen.run(output_path).await.unwrap();
    }

    log::info!("Load generator done");
    Ok(())
}

async fn stats_logger(counters: Arc<Counters>, pause_at: Instant) {
    let mut secs = 0;
    let mut prev = Counters::new(&DEFAULT_COUNTER_KEYS);
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
