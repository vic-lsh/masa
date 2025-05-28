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
use std::path::Path;
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver, Sender};
use structopt::StructOpt;
use tokio::task::JoinSet;
use tokio::time::{timeout, Duration, Instant};
use tonic::transport::Channel;

use app_utils::{
    load_gen::{Counters, GenConfig, LoadGenArgs},
    logging::init_logging_file,
    timing::time_now,
};
use frontend::frontend_client::FrontendClient;
use masa::Context;

const COUNTER_KEYS: [&'static str; 7] = [
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
    // total number of errors in API 'a'
    "err_a",
    // total number of unexpected errors
    "unexpected",
];

#[derive(Debug, Clone)]
struct RequestStats {
    ctx: Context,
    latency: u64,
    frontend_latency: u64,
    child1_latency: u64,
    child2_latency: u64,
    error: String,
}

impl RequestStats {
    const HEADERS: [&'static str; 12] = [
        "api",
        "test_id",
        "request_id",
        "slo",
        "request_class",
        "start_at",
        "deadline",
        "latency",
        "error",
        "frontend_latency",
        "child1_latency",
        "child2_latency",
    ];

    fn new(
        ctx: Context,
        latency: u64,
        response: Option<frontend::AResponse>,
        error: String,
    ) -> Self {
        let mut s = Self {
            ctx,
            latency,
            frontend_latency: 0,
            child1_latency: 0,
            child2_latency: 0,
            error,
        };

        if let Some(r) = response {
            s.frontend_latency = r.handler_latency;
            s.child1_latency = r.child1_latency;
            s.child2_latency = r.child2_latency;
        }

        s
    }

    fn to_row(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            self.ctx.api(),
            self.ctx.test_id(),
            self.ctx.request_id(),
            self.ctx.slo(),
            self.ctx.request_class(),
            self.ctx.start_at(),
            self.ctx.deadline(),
            self.latency,
            self.error,
            self.frontend_latency,
            self.child1_latency,
            self.child2_latency,
        )
    }

    fn header_row() -> String {
        Self::HEADERS.join(",")
    }
}

async fn fetch_traces(output_file: String, trace_rx: Receiver<RequestStats>) {
    let path = Path::new(&output_file);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).unwrap();
        }
    }
    let mut file = File::create(output_file).unwrap();
    writeln!(file, "{}", RequestStats::header_row()).unwrap();
    while let Ok(stats) = trace_rx.recv() {
        writeln!(file, "{}", stats.to_row()).unwrap();
    }
    log::warn!("All traces fetched");
}

#[derive(Debug)]
struct LoadGenerator {
    gen_cfg: GenConfig,
    rps: u64,
    client: FrontendClient<Channel>,
    trace_tx: Sender<RequestStats>,
}

impl LoadGenerator {
    pub fn new(
        gen_cfg: GenConfig,
        rps: u64,
        client: FrontendClient<Channel>,
        trace_tx: Sender<RequestStats>,
    ) -> Self {
        Self {
            gen_cfg,
            rps,
            client,
            trace_tx,
        }
    }

    async fn run(&mut self) -> Result<(), Box<dyn Error>> {
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

        log::warn!("Load generated");
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
        let mut counter_test_id = 0;
        let mut elapse = 0f64;

        let mut set = JoinSet::new();

        while Instant::now() < pause_at {
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

            let api = self.gen_cfg.apis[0].clone();
            let slo = self.gen_cfg.slos[0];

            let ctx = {
                let test_id = counter_test_id;
                counter_test_id += 1;
                let request_id = 0;
                let request_class = 0;

                let start_at = time_now();
                let deadline = start_at + slo;

                Context::new(
                    api.clone(),
                    test_id,
                    request_id,
                    slo,
                    request_class,
                    start_at,
                    deadline,
                )
            };

            let mut client = self.client.clone();
            let trace_tx = self.trace_tx.clone();
            let ctrs = Arc::clone(&counters);

            set.spawn(async move {
                ctrs.increment("all");
                let stats = send_request(&mut client, &api, ctx).await;

                if Instant::now() < trace_at {
                    return;
                }

                // increment the right counters
                let error = &stats.error;
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

                if error != "/None" {
                    match api.as_str() {
                        "a" => {
                            ctrs.increment("err_a");
                        }
                        _ => panic!("should never happen"),
                    }
                }

                trace_tx.try_send(stats).unwrap();
            });
        }

        // wait for all outgoing requests to complete
        while let Some(_) = set.join_next().await {}
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = LoadGenArgs::from_args();

    let path = Path::new(&args.output_path);
    if !path.exists() {
        fs::create_dir_all(path).unwrap();
    }

    init_logging_file(&args.output_path);

    let gen_cfg: GenConfig = {
        let file = File::open(args.gen_config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };
    assert!(gen_cfg.gap == "const" || gen_cfg.gap == "exp");
    log::warn!("Gen config: {:?}", gen_cfg);

    for rps in &gen_cfg.rps_values {
        log::warn!("Running rps: {}...", rps);

        let output = format!("{}/r{}.csv", args.output_path, rps);
        let (trace_tx, trace_rx) = unbounded();

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

            let load_gen = LoadGenerator::new(gen_cfg.clone(), *rps, client, trace_tx);
            load_gen
        };

        let mut handles = Vec::new();
        handles.push(tokio::spawn(async move {
            load_gen.run().await.unwrap();
        }));
        handles.push(tokio::spawn(async move {
            fetch_traces(output, trace_rx).await;
        }));
        for h in handles {
            h.await.unwrap();
        }
    }

    log::warn!("Load generator done");
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
            "total early returns: {}, total deadline misses: {}, total timeouts: {}, total a errors: {}, total unexpected errors: {}",
            counters.get("err_svc_er"),
            counters.get("err_cl_miss"),
            counters.get("err_cl_to"),
            counters.get("err_a"),
            counters.get("unexpected"),
        );
        // clone the Counters struct itself as opposed to creating another reference
        prev = (*counters).clone();
    }
}

async fn send_request(
    client: &mut FrontendClient<Channel>,
    api: &str,
    ctx: Context,
) -> RequestStats {
    let send_at;
    let recv_at;
    let timeout_duration = Duration::from_secs(1);

    let response = match api {
        "a" => {
            let request = {
                let mut request = tonic::Request::new(frontend::ARequest {});
                request.metadata_mut().insert_ctx("ctx", &ctx);
                request
            };
            send_at = time_now();
            let r = timeout(timeout_duration, client.handle_a(request)).await;
            recv_at = time_now();
            r
        }
        _ => panic!("Unimplemented API {}", api),
    };
    let stats = match response {
        Ok(response) => {
            let latency = recv_at - send_at;
            let error = {
                if let Err(ref status) = response {
                    status.message().to_string()
                } else if latency > ctx.slo() {
                    "/ClientMiss".to_string()
                } else {
                    "/None".to_string()
                }
            };
            RequestStats::new(ctx, latency, response.map(|r| r.into_inner()).ok(), error)
        }
        Err(_) => {
            let error = "/ClientTimeout".to_string();
            RequestStats::new(ctx, timeout_duration.as_micros() as u64, None, error)
        }
    };

    stats
}
