use crossbeam_channel::Receiver;
use masa::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use structopt::StructOpt;
use tokio::time::error::Elapsed;
use tonic::Status;

#[derive(Debug, Clone)]
pub struct RequestStats {
    pub ctx: Context,
    pub latency: u64,
    pub error: String,
}

impl RequestStats {
    pub fn new(ctx: Context, latency: u64, error: String) -> Self {
        Self {
            ctx,
            latency,
            error,
        }
    }
}

pub async fn fetch_traces(output_file: String, trace_rx: Receiver<RequestStats>) {
    let path = Path::new(&output_file);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).unwrap();
        }
    }
    let mut file = File::create(output_file).unwrap();
    writeln!(
        file,
        "api,test_id,request_id,slo,request_class,start_at,deadline,latency,error"
    )
    .unwrap();
    while let Ok(span) = trace_rx.recv() {
        writeln!(
            file,
            "{},{},{},{},{},{},{},{},{}",
            span.ctx.api(),
            span.ctx.test_id(),
            span.ctx.request_id(),
            span.ctx.slo(),
            span.ctx.request_class(),
            span.ctx.start_at(),
            span.ctx.deadline(),
            span.latency,
            span.error
        )
        .unwrap();
    }
    log::warn!("All traces fetched");
}

pub fn map_response<T>(
    timeout_response: Result<Result<T, Status>, Elapsed>,
) -> Result<Result<(), Status>, Elapsed> {
    timeout_response.map(|response| response.map(|_r| ()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenConfig {
    #[serde(rename = "Repeats")]
    pub repeats: u64,
    #[serde(rename = "Apis")]
    pub apis: Vec<String>,
    #[serde(rename = "Slos")]
    pub slos: Vec<u64>,
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
            .unwrap()
            .fetch_add(1, Ordering::SeqCst);
    }
}

// #[derive(Debug)]
// struct LoadGenerator<C> {
//     gen_cfg: GenConfig,
//     rng: StdRng,
//     rps: u64,
//     client: C,
//     trace_tx: Sender<RequestStats>,
// }
//
// impl LoadGenerator {
//     pub fn new(
//         gen_cfg: GenConfig,
//         rng: StdRng,
//         rps: u64,
//         client: FrontendClient<Channel>,
//         trace_tx: Sender<RequestStats>,
//     ) -> Self {
//         Self {
//             gen_cfg,
//             rng,
//             rps,
//             client,
//             trace_tx,
//         }
//     }
//
//     async fn run(&mut self) -> Result<(), Box<dyn Error>> {
//         let init_at = Instant::now();
//         let warm_at = init_at + Duration::from_secs_f64(self.gen_cfg.warmup_secs as f64 / 2.0);
//         let trace_at = init_at + Duration::from_secs(self.gen_cfg.warmup_secs);
//         let pause_at =
//             init_at + Duration::from_secs(self.gen_cfg.warmup_secs + self.gen_cfg.duration_secs);
//
//         let counters = Arc::new(Counters::new());
//
//         let h = tokio::task::spawn(stats_logger(Arc::clone(&counters), pause_at));
//
//         self.generate_load(counters, init_at, trace_at, warm_at, pause_at)
//             .await;
//
//         let _ = h.await;
//
//         log::warn!("Load generated");
//         Ok(())
//     }
//
//     async fn generate_load(
//         &mut self,
//         counters: Arc<Counters>,
//         init_at: Instant,
//         trace_at: Instant,
//         warm_at: Instant,
//         pause_at: Instant,
//     ) {
//         let mut counter_test_id = 0;
//         let exponential = Exp::new(self.rps as f64).unwrap();
//         let mut elapse = 0f64;
//         let uniform = Uniform::<u32>::new(0, 1_000_000_007);
//
//         let mut set = JoinSet::new();
//
//         while Instant::now() < pause_at {
//             let start_at = init_at + Duration::from_secs_f64(elapse);
//             tokio::time::sleep_until(start_at).await;
//
//             let value = {
//                 if Instant::now() < warm_at {
//                     0.01
//                 } else {
//                     if self.gen_cfg.gap == "const" {
//                         1f64 / self.rps as f64
//                     } else {
//                         exponential.sample(&mut self.rng)
//                     }
//                 }
//             };
//             elapse += value;
//
//             let api_idx = uniform.sample(&mut self.rng) as usize % self.gen_cfg.apis.len();
//             let api = self.gen_cfg.apis[api_idx].clone();
//             let slo = self.gen_cfg.slos[api_idx];
//
//             let ctx = {
//                 let test_id = counter_test_id;
//                 counter_test_id += 1;
//                 let request_id = uniform.sample(&mut self.rng) as u64;
//                 let request_class = 0;
//
//                 let start_at = time_now();
//                 let deadline = start_at + slo;
//
//                 Context::new(
//                     api.clone(),
//                     test_id,
//                     request_id,
//                     slo,
//                     request_class,
//                     start_at,
//                     deadline,
//                 )
//             };
//
//             let mut client = self.client.clone();
//             let trace_tx = self.trace_tx.clone();
//             let ctrs = Arc::clone(&counters);
//
//             set.spawn(async move {
//                 ctrs.increment("all");
//                 let stats = send_request(&mut client, &api, ctx).await;
//
//                 if Instant::now() < trace_at {
//                     return;
//                 }
//
//                 // increment the right counters
//                 let error = &stats.error;
//                 match error.as_str() {
//                     "/None" => {
//                         ctrs.increment("good");
//                     }
//                     "/ClientMiss" => {
//                         ctrs.increment("err_cl_miss");
//                     }
//                     "/EarlyReturn" => {
//                         ctrs.increment("err_svc_er");
//                     }
//                     "/ClientTimeout" => {
//                         ctrs.increment("err_cl_to");
//                     }
//                     e => {
//                         ctrs.increment("unexpected");
//                         log::error!("unexpected request error '{}'", e);
//                     }
//                 };
//
//                 if error != "/None" {
//                     match api.as_str() {
//                         "Search" => {
//                             ctrs.increment("err_search");
//                         }
//                         "Reservation" => {
//                             ctrs.increment("err_reservation");
//                         }
//                         _ => panic!("should never happen"),
//                     }
//                 }
//
//                 trace_tx.try_send(stats).unwrap();
//             });
//         }
//
//         // wait for all outgoing requests to complete
//         while let Some(_) = set.join_next().await {}
//     }
// }
//
// async fn start_load_gen<K, S, C, F, L>(connect: K, send_api_request: S) -> Result<(), Box<dyn std::error::Error>>
// where
//     K: Fn(String) -> C,
//     S: Fn(C, String) -> F,
//     F: Future<Output = Result<(), Status>>,
// {
//     init_logging();
//
//     let args = Args::from_args();
//     let gen_cfg: GenConfig = {
//         let file = File::open(args.gen_config).expect("Failed to open file");
//         let reader = BufReader::new(file);
//         serde_json::from_reader(reader)?
//     };
//     assert!(gen_cfg.gap == "const" || gen_cfg.gap == "exp");
//     log::warn!("Gen config: {:?}", gen_cfg);
//
//     for rps in &gen_cfg.rps_values {
//         log::warn!("Running rps: {}...", rps);
//
//         let output = format!("{}/r{}.csv", args.output_path, rps);
//         let (trace_tx, trace_rx) = unbounded();
//
//         let mut load_gen = {
//             const KEY: u64 = 13;
//             const SEED: u64 = 998244353;
//
//             let seed = SEED * KEY + rps;
//             let rng = StdRng::seed_from_u64(seed);
//             let client = {
//                 let mut client = FrontendClient::connect(gen_cfg.addr.clone()).await?;
//                 let ctx = {
//                     let test_id = 0;
//                     let request_id = 0;
//                     let slo = 1_000_000;
//                     let request_class = 0;
//                     let start_at = time_now();
//                     let deadline = start_at + slo;
//                     Context::new(
//                         "Ping".to_string(),
//                         test_id,
//                         request_id,
//                         slo,
//                         request_class,
//                         start_at,
//                         deadline,
//                     )
//                 };
//                 request.metadata_mut().insert_ctx("ctx", &ctx);
//                 send
//                 client.handle_ping(request).await?;
//                 client
//             };
//
//             let load_gen = LoadGenerator::new(gen_cfg.clone(), rng, *rps, client, trace_tx);
//             load_gen
//         };
//
//         let mut handles = Vec::new();
//         handles.push(tokio::spawn(async move {
//             load_gen.run().await.unwrap();
//         }));
//         handles.push(tokio::spawn(async move {
//             fetch_traces(output, trace_rx).await;
//         }));
//         for h in handles {
//             h.await.unwrap();
//         }
//     }
//
//     log::warn!("Load generator done");
//     Ok(())
// }
//
// async fn stats_logger(counters: Arc<Counters>, pause_at: Instant) {
//     let mut secs = 0;
//     let mut prev = Counters::new();
//     while Instant::now() < pause_at {
//         tokio::time::sleep(Duration::from_secs(1)).await;
//         secs += 1;
//
//         let delta = |k| counters.get(k) - prev.get(k);
//
//         log::warn!(
//             "secs: {}, rps: {}, goodput: {}, early returns: {}, deadline misses: {}, timeouts: {}",
//             secs,
//             delta("all"),
//             delta("good"),
//             delta("err_svc_er"),
//             delta("err_cl_miss"),
//             delta("err_cl_to"),
//         );
//         log::warn!(
//             "total early returns: {}, total deadline misses: {}, total timeouts: {}, total search errors: {}, total reservation errors: {}, total unexpected errors: {}",
//             counters.get("err_svc_er"),
//             counters.get("err_cl_miss"),
//             counters.get("err_cl_to"),
//             counters.get("err_search"),
//             counters.get("err_reservation"),
//             counters.get("unexpected"),
//         );
//         // clone the Counters struct itself as opposed to creating another reference
//         prev = (*counters).clone();
//     }
// }
//
// async fn send_request(
//     client: &mut FrontendClient<Channel>,
//     api: &str,
//     ctx: Context,
// ) -> RequestStats {
//     let send_at;
//     let recv_at;
//     let timeout_duration = Duration::from_secs(1);
//
//     let response = match api {
//         "Search" => {
//             let request = {
//                 let mut request = tonic::Request::new(get_search_request());
//                 request.metadata_mut().insert_ctx("ctx", &ctx);
//                 request
//             };
//             send_at = time_now();
//             let r = timeout(timeout_duration, client.handle_search(request)).await;
//             recv_at = time_now();
//             map_response(r)
//         }
//         "Reservation" => {
//             let request = {
//                 let mut request = tonic::Request::new(get_reservation_request());
//                 request.metadata_mut().insert_ctx("ctx", &ctx);
//                 request
//             };
//             send_at = time_now();
//             let r = timeout(timeout_duration, client.handle_reservation(request)).await;
//             recv_at = time_now();
//             map_response(r)
//         }
//         _ => panic!("Unimplemented API {}", api),
//     };
//     let stats = match response {
//         Ok(response) => {
//             let latency = recv_at - send_at;
//             let error = {
//                 if let Err(ref status) = response {
//                     status.message().to_string()
//                 } else if latency > ctx.slo() {
//                     "/ClientMiss".to_string()
//                 } else {
//                     "/None".to_string()
//                 }
//             };
//             RequestStats::new(ctx, latency, error)
//         }
//         Err(_) => {
//             let error = "/ClientTimeout".to_string();
//             RequestStats::new(ctx, timeout_duration.as_micros() as u64, error)
//         }
//     };
//
//     stats
// }
