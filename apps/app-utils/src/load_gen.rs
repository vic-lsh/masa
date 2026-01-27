use log::info;
use log::warn;
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::fs::File;
use std::future::Future;
use std::io::BufReader;
use std::io::Write;
use std::iter::zip;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::Semaphore;
use tokio::time::{timeout, Duration, Instant};

use rand::distributions::{Distribution as RandDistribution, WeightedIndex};
use rand_distr::{Distribution, Exp};
use structopt::StructOpt;
use tokio::task::JoinSet;
use tokio::time::error::Elapsed;
use tonic::metadata::MetadataMap;

use crate::{
    logging::init_logging_file,
    timing::{get_timestamp, time_now},
};
use masa::{Context, ContextBuilder, PriorityHint};
use tonic::Response;
use tonic::Status;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArrivalProcess {
    /// Constant inter-arrival times (fixed interval)
    Const,
    /// Exponential inter-arrival times (Poisson arrival process)
    Exp,
}

/// Manages the arrival process timing, hiding details about the specific
/// arrival process type (constant vs exponential) and RPS.
pub struct ArrivalTimer {
    arrival_process: ArrivalProcess,
    rps: u64,
    warm_at: Instant,
    exp_dist: Option<Exp<f64>>,
    rng: StdRng,
    elapse: f64,
}

impl ArrivalTimer {
    /// Create a new arrival timer with the specified arrival process, RPS, warmup time, and RNG seed.
    pub fn new(arrival_process: ArrivalProcess, rps: u64, warm_at: Instant, rng: StdRng) -> Self {
        let exp_dist = match arrival_process {
            ArrivalProcess::Exp => {
                Some(Exp::new(rps as f64).expect("Failed to create exponential distribution"))
            }
            ArrivalProcess::Const => None,
        };

        Self {
            arrival_process,
            rps,
            warm_at,
            exp_dist,
            rng,
            elapse: 0.0,
        }
    }

    /// Advance the timer and return the next inter-arrival time in seconds.
    /// During warmup, returns a fixed small interval for faster ramp-up.
    pub fn tick(&mut self) -> f64 {
        let now = Instant::now();
        let interval = if now < self.warm_at {
            // During warmup, use fixed small interval for faster ramp-up
            0.01
        } else {
            match self.arrival_process {
                ArrivalProcess::Const => {
                    // Constant inter-arrival time (fixed interval)
                    1f64 / self.rps as f64
                }
                ArrivalProcess::Exp => {
                    // Sample exponential inter-arrival time for Poisson process
                    // The exponential distribution with rate λ has mean 1/λ
                    self.exp_dist
                        .as_ref()
                        .expect("Exponential distribution should be initialized")
                        .sample(&mut self.rng)
                }
            }
        };
        self.elapse += interval;
        interval
    }

    /// Get the time offset for the next arrival in seconds (relative to start time).
    pub fn next_arrival_time(&self) -> f64 {
        self.elapse
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiSpec {
    #[serde(rename = "Name", alias = "name")]
    pub name: String,
    #[serde(rename = "Weight", alias = "weight", default = "default_api_weight")]
    pub weight: f64,
}

fn default_api_weight() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ApiEntry {
    Name(String),
    Spec(ApiSpec),
}

impl ApiEntry {
    pub fn name(&self) -> &str {
        match self {
            Self::Name(name) => name,
            Self::Spec(spec) => &spec.name,
        }
    }

    pub fn weight(&self) -> f64 {
        match self {
            Self::Name(_) => 1.0,
            Self::Spec(spec) => spec.weight,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenConfig {
    #[serde(rename = "Repeats")]
    pub repeats: u64,
    #[serde(rename = "Apis")]
    pub apis: Vec<ApiEntry>,
    #[serde(rename = "Slos")]
    pub slos: Vec<u64>,
    #[serde(rename = "Timeouts_ms")]
    pub timeouts_ms: Vec<u64>,
    #[serde(rename = "Rps")]
    pub rps_values: Vec<u64>,
    #[serde(rename = "Gap")]
    pub gap: ArrivalProcess,
    #[serde(rename = "WarmupSecs")]
    pub warmup_secs: u64,
    #[serde(rename = "DurationSecs")]
    pub duration_secs: u64,
    #[serde(rename = "MaxInFlight")]
    #[serde(default = "default_max_in_flight")]
    pub max_in_flight: usize,
    #[serde(rename = "Addr")]
    pub addr: String,
}

fn default_max_in_flight() -> usize {
    0 // 0 means unlimited
}

fn expand_config_values(
    values: &[u64],
    count: usize,
    label: &str,
) -> Result<Vec<u64>, Box<dyn std::error::Error>> {
    match values.len() {
        0 => Err(format!("{label} must not be empty").into()),
        1 => Ok(vec![values[0]; count]),
        n if n == count => Ok(values.to_vec()),
        n => Err(format!(
            "{label} length ({n}) must be 1 or match Apis length ({count})"
        )
        .into()),
    }
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
    "early_return",
    // number of deadline misses without timing out
    "deadline_miss",
    // number of timeouts
    "timeout",
    // total number of unexpected errors
    "unexpected",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FutureSpanType {
    Compute,
    Block,
}

#[derive(Debug, Clone, Copy)]
pub struct FutureSpan {
    pub kind: FutureSpanType,
    pub duration: Duration,
}

#[derive(Debug)]
pub struct Task {
    pub id: u64,
    pub start_at: u64,
    pub spans: Vec<FutureSpan>,
}

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
pub struct RequestStats<R, C>
where
    R: RequestType<C>,
    C: Client,
{
    ctx: Context,
    latency: u64,
    error: String,
    response: Option<(MetadataMap, R::ResponseType)>,
}

impl<R, C> RequestStats<R, C>
where
    R: RequestType<C>,
    C: Client,
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
        response: Option<(MetadataMap, R::ResponseType)>,
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
            Some((m, r)) => R::response_to_row(&m, &r).join(","),
            None => "".to_string(),
        };

        format!("{},{}", generic, specific)
    }
}

pub struct Handler<R, C>
where
    R: RequestType<C>,
    C: Client,
{
    pub api: String,
    pub rps: u64,
    pub timeout: Duration,
    pub slo: u64,

    inner: R,
    trace_tx: Option<UnboundedSender<RequestStats<R, C>>>,
    trace_rx: UnboundedReceiver<RequestStats<R, C>>,
}

impl<R, C> Handler<R, C>
where
    R: RequestType<C>,
    C: Client,
{
    pub fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            inner: R::new(api),
            api: api.to_string(),
            rps,
            timeout,
            slo,

            trace_tx: Some(tx),
            trace_rx: rx,
        }
    }

    pub async fn send_request(
        &self,
        mut rng: StdRng,
        client: C::FrontendClient,
        ctx: Context,
        trace: bool,
    ) -> String {
        let latency;
        let response = {
            let send_at = time_now();
            let r = timeout(
                self.timeout,
                self.inner.create_request(&mut rng, client, &ctx),
            )
            .await;
            let recv_at = time_now();
            latency = recv_at - send_at;
            r
        };

        let (response, error) = map_response(response, latency <= self.slo);

        let stats = RequestStats::new(ctx, latency, error.clone(), response);

        if trace {
            self.trace_tx.as_ref().unwrap().send(stats).unwrap();
        }

        error
    }

    pub async fn fetch_traces(&mut self, output_path: &Path) {
        // must drop so that channel closes
        {
            self.trace_tx.take()
        };
        let safe_api = self.api.replace('/', "_").replace(' ', "_");
        let mut file =
            File::create(output_path.join(format!("r{}_{}.csv", self.rps, safe_api))).unwrap();
        writeln!(file, "{}", self.header_row()).unwrap();
        while let Some(stats) = self.trace_rx.recv().await {
            writeln!(file, "{}", stats.to_row()).unwrap();
        }
        log::info!("All traces fetched");
    }

    pub fn header_row(&self) -> String {
        let generic = RequestStats::<R, C>::HEADERS.join(",");
        let specific = self.inner.response_output_headers().join(",");
        format!("{},{}", generic, specific)
    }
}

pub trait RequestType<C>
where
    Self: Sized,
    C: Client,
{
    type ResponseType;

    fn new(api: &str) -> Self;

    fn create_request(
        &self,
        rng: &mut StdRng,
        client: C::FrontendClient,
        ctx: &Context,
    ) -> impl Future<Output = Result<Response<Self::ResponseType>, Status>>;

    fn response_output_headers(&self) -> Vec<String>;

    fn response_to_row(metadata: &MetadataMap, response: &Self::ResponseType) -> Vec<String>;
}

#[allow(async_fn_in_trait)]
pub trait Client
where
    Self::FrontendClient: Send + Clone + 'static,
    Self: 'static,
{
    type FrontendClient;

    async fn connect(dst: String) -> Result<Self::FrontendClient, tonic::transport::Error>;

    async fn ping(client: &mut Self::FrontendClient) -> Result<(), tonic::Status>;

    async fn connect_with_retry(
        dst: String,
    ) -> Result<Self::FrontendClient, tonic::transport::Error> {
        const MAX_RETRIES: usize = 5;
        let mut retries = 0;
        let mut backoff_secs = 1;
        loop {
            match Self::connect(dst.clone()).await {
                Ok(c) => return Ok(c),
                Err(e) => {
                    retries += 1;
                    if retries == MAX_RETRIES {
                        return Err(e);
                    }
                    warn!(
                        "Failed to connect to {}; {}/{} attempts; {}",
                        dst, retries, MAX_RETRIES, e
                    );

                    tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                    backoff_secs *= 2;
                }
            };
        }
    }
}

#[allow(async_fn_in_trait)]
pub trait HandlerOuter<C>
where
    C: Client,
    Self: Send + Sync + 'static,
{
    fn new(api: &str, rps: u64, timeout: Duration, slo: u64) -> Self;

    fn send_request(
        &self,
        rng: StdRng,
        client: C::FrontendClient,
        ctx: Context,
        trace: bool,
    ) -> impl Future<Output = String> + Send;

    async fn fetch_traces(&mut self, output_path: &Path);

    fn api(&self) -> &str;

    fn slo(&self) -> u64;
}

pub struct LoadGenerator<H, C>
where
    H: HandlerOuter<C>,
    C: Client,
{
    rng: StdRng,
    gen_cfg: GenConfig,
    rps: u64,
    client: C::FrontendClient,
    api_handlers: Vec<Arc<H>>,
    api_weight_index: WeightedIndex<f64>,
}

impl<H, C> LoadGenerator<H, C>
where
    H: HandlerOuter<C> + Send + Sync,
    C: Client,
{
    pub fn new(
        seed: u64,
        gen_cfg: GenConfig,
        rps: u64,
        client: C::FrontendClient,
        api_handlers: Vec<Arc<H>>,
        api_weight_index: WeightedIndex<f64>,
    ) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed + rps),
            gen_cfg,
            rps,
            client,
            api_handlers,
            api_weight_index,
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

        let mut set = JoinSet::new();
        let max_in_flight = self.gen_cfg.max_in_flight;

        // Create semaphore for max in-flight control only if max_in_flight > 0
        // If max_in_flight is 0 (unlimited), don't use a semaphore at all
        let inflight_guard: Option<Arc<Semaphore>> = if max_in_flight > 0 {
            Some(Arc::new(Semaphore::new(max_in_flight)))
        } else {
            None
        };

        // Create arrival timer to manage inter-arrival times
        let mut arrival_timer =
            ArrivalTimer::new(self.gen_cfg.gap, self.rps, warm_at, self.rng.clone());

        while Instant::now() < pause_at {
            // XXX: tokio's sleep has millisecond granularity, so for small intervals this may be
            // inaccurate
            let start_at = init_at + Duration::from_secs_f64(arrival_timer.next_arrival_time());
            tokio::time::sleep_until(start_at).await;

            // Advance timer and get next inter-arrival time
            // This updates elapse internally, so we advance even if we skip this request
            let _interval = arrival_timer.tick();

            // Try to acquire permit for max-in-flight control if semaphore exists
            // If acquisition fails, skip this request and continue to next iteration
            let permit = if let Some(ref guard) = inflight_guard {
                match guard.clone().try_acquire_owned() {
                    Ok(p) => Some(p),
                    Err(_) => continue,
                }
            } else {
                None
            };

            let i = self.api_weight_index.sample(&mut self.rng);
            let handler = Arc::clone(&self.api_handlers[i]);

            let ctx = {
                let request_id = counter_request_id;
                counter_request_id += 1;

                let start_at = time_now();
                let deadline = start_at + handler.slo();
                let prio_hint = if masa::PRIO_OLDEST {
                    start_at
                } else {
                    deadline
                };

                ContextBuilder::new(handler.api().to_string(), request_id)
                    .slo(handler.slo())
                    .start_at(start_at)
                    .deadline(deadline)
                    .prio_hint(PriorityHint::new(prio_hint))
                    .build()
            };

            let client = self.client.clone();
            let ctrs = Arc::clone(&counters);
            let rng = self.rng.clone();
            let trace = Instant::now() > trace_at;

            set.spawn(async move {
                let _permit = permit; // Hold permit until task completes
                ctrs.increment("all");

                let error = handler.send_request(rng, client, ctx, trace).await;

                if trace {
                    // increment the right counters
                    if error == "/None" {
                        ctrs.increment("good");
                    } else if error == "/ClientMiss" {
                        ctrs.increment("deadline_miss");
                    } else if error == "/ClientTimeout" {
                        ctrs.increment("timeout");
                    } else if error.starts_with("/EarlyReturn") {
                        ctrs.increment("early_return");
                    } else {
                        ctrs.increment("unexpected");
                        log::error!("unexpected request error '{}'", error);
                    }
                }
            });
        }

        // wait for all outgoing requests to complete
        while let Some(_) = set.join_next().await {}
    }
}

pub async fn load_gen_main<H, C>(
    args: LoadGenArgs,
    seed: u64,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: HandlerOuter<C>,
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

    for rps in &gen_cfg.rps_values {
        log::info!("Running rps: {}... ({})", rps, get_timestamp());

        if gen_cfg.apis.is_empty() {
            return Err("Apis must not be empty".into());
        }
        let slos = expand_config_values(&gen_cfg.slos, gen_cfg.apis.len(), "Slos")?;
        let timeouts_ms =
            expand_config_values(&gen_cfg.timeouts_ms, gen_cfg.apis.len(), "Timeouts_ms")?;
        let mut api_handlers = Vec::new();
        for (api, (timeout_ms, slo)) in zip(&gen_cfg.apis, zip(&timeouts_ms, &slos))
        {
            api_handlers.push(Arc::new(H::new(
                api.name(),
                *rps,
                Duration::from_millis(*timeout_ms),
                *slo,
            )));
        }

        log::info!("Before Connection");

        let mut load_gen = {
            let client = {
                let mut client = C::connect_with_retry(gen_cfg.addr.clone())
                    .await
                    .expect(&format!("Should be able to connect to {}", gen_cfg.addr));
                C::ping(&mut client)
                    .await
                    .expect("Should be able to ping to client");
                info!("Connected to {}", gen_cfg.addr);
                client
            };

            log::info!("Connected to {}", gen_cfg.addr);

            let weights: Vec<f64> = gen_cfg.apis.iter().map(|api| api.weight()).collect();
            if weights.iter().any(|w| *w <= 0.0) {
                return Err("All API weights must be > 0".into());
            }
            let api_weight_index = WeightedIndex::new(&weights)?;
            let load_gen = LoadGenerator::new(
                seed,
                gen_cfg.clone(),
                *rps,
                client,
                api_handlers,
                api_weight_index,
            );
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
            "secs: {}, rps: {}, good: {}, ER: {}, ddl_miss: {}, timeouts: {}; total: ER {}, ddl_miss {}, timeout {}",
            secs,
            delta("all"),
            delta("good"),
            delta("early_return"),
            delta("deadline_miss"),
            delta("timeout"),
            counters.get("early_return"),
            counters.get("deadline_miss"),
            counters.get("timeout"),
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

// pub fn parse_tasks_from_file<P: AsRef<Path>>(path: P) -> io::Result<Vec<Task>> {
//     let file = File::open(path)?;
//     let reader = BufReader::new(file);

//     reader
//         .lines()
//         .enumerate()
//         .map(|(i, line_result)| {
//             let line = line_result?;
//             parse_line(&line).map_err(|e| {
//                 io::Error::new(io::ErrorKind::InvalidData, format!("Error on line {}: {}", i + 1, e))
//             })
//         })
//         .collect()
// }

// fn parse_line(line: &str) -> Result<Task, String> {
//     let mut parts = line.split(',');
//     let id_str = parts.next().ok_or("Line is empty")?;
//     let id = id_str
//         .parse::<u64>()
//         .map_err(|e| format!("Invalid Task ID '{}': {}", id_str, e))?;

//     let mut spans = Vec::new();
//     for part in parts {-
//         let (type_str, duration_part) = part
//             .split('(')
//             .ok_or(format!("Malformed stage: '{}'", part))?;
//         let kind = match type_str {
//             "Compute" => FutureSpanType::Compute,
//             "Block" => FutureSpanType::Block,
//             _ => return Err(format!("Unknown stage type: '{}'", type_str)),
//         };
//         let duration_str = duration_part
//             .strip_suffix("us)")
//             .ok_or(format!("Malformed duration: '{}'", duration_part))?;
//         let micros = duration_str
//             .parse::<u64>()
//             .map_err(|e| format!("Invalid duration value '{}': {}", duration_str, e))?;
//         spans.push(FutureSpan {
//             kind,
//             duration: Duration::from_micros(micros),
//         });
//     }
//     Ok(Task { id, spans })
// }
