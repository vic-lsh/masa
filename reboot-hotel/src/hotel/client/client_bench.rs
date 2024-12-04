#[path = "../config.rs"]
pub mod config;
pub mod hotel {
    tonic::include_proto!("frontend");
}
mod gen;

use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use crossbeam_channel::{unbounded, Sender};
use gen::{get_ping_request, get_reservation_request, get_search_request};
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp, Uniform};
use structopt::StructOpt;
use tokio::time::{timeout, Duration, Instant};

use tonic::transport::Channel;
use tonic_masa::Context;

use config::{GenConfig, HotelConfig};
use hotel::frontend_client::FrontendClient;
use reboot_hotel::{fetch_traces, init_logging, time_now, Span};

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
    #[structopt(long, required = true)]
    pub hotel_config: PathBuf,
    #[structopt(long, required = true)]
    pub gen_config: PathBuf,
    #[structopt(long, required = true)]
    pub output_path: String,
    #[structopt(long, required = true)]
    pub run_idx: String,
}

#[derive(Debug)]
struct LoadGenerator {
    _hotel_cfg: HotelConfig,
    gen_cfg: GenConfig,
    rng: StdRng,
    rps: u64,
    client: FrontendClient<Channel>,
    trace_tx: Sender<Span>,
}

impl LoadGenerator {
    pub fn new(
        _hotel_cfg: HotelConfig,
        gen_cfg: GenConfig,
        rng: StdRng,
        rps: u64,
        client: FrontendClient<Channel>,
        trace_tx: Sender<Span>,
    ) -> Self {
        Self {
            _hotel_cfg,
            gen_cfg,
            rng,
            rps,
            client,
            trace_tx,
        }
    }

    async fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let init_at = Instant::now();
        // let init_at_u64 = time_now();
        let warm_at = init_at + Duration::from_secs(self.gen_cfg.warmup_secs / 2);
        let trace_at = init_at + Duration::from_secs(self.gen_cfg.warmup_secs);
        let pause_at =
            init_at + Duration::from_secs(self.gen_cfg.warmup_secs + self.gen_cfg.duration_secs);

        let cnt_all = Arc::new(AtomicUsize::new(0));
        let cnt_good = Arc::new(AtomicUsize::new(0));
        let cnt_err_svc_er = Arc::new(AtomicUsize::new(0));
        let cnt_err_cl_miss = Arc::new(AtomicUsize::new(0));
        let cnt_err_cl_to = Arc::new(AtomicUsize::new(0));

        let cnt_err_search = Arc::new(AtomicUsize::new(0));
        let cnt_err_reservation = Arc::new(AtomicUsize::new(0));

        let cnt_all_clone = cnt_all.clone();
        let cnt_good_clone = cnt_good.clone();
        let cnt_err_svc_er_clone = cnt_err_svc_er.clone();
        let cnt_err_cl_miss_clone = cnt_err_cl_miss.clone();
        let cnt_err_cl_to_clone = cnt_err_cl_to.clone();

        let cnt_err_search_clone = cnt_err_search.clone();
        let cnt_err_reservation_clone = cnt_err_reservation.clone();

        tokio::task::spawn(async move {
            let mut all_prev = 0;
            let mut good_prev = 0;
            let mut err_svc_er_prev = 0;
            let mut err_cl_miss_prev = 0;
            let mut err_cl_to_prev = 0;
            let mut secs = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let all = cnt_all_clone.load(Ordering::Relaxed);
                let good = cnt_good_clone.load(Ordering::Relaxed);
                let err_svc_er = cnt_err_svc_er_clone.load(Ordering::Relaxed);
                let err_cl_miss = cnt_err_cl_miss_clone.load(Ordering::Relaxed);
                let err_cl_to = cnt_err_cl_to_clone.load(Ordering::Relaxed);
                let err_search = cnt_err_search_clone.load(Ordering::Relaxed);
                let err_reserve = cnt_err_reservation_clone.load(Ordering::Relaxed);

                secs += 1;

                let rps = all - all_prev;
                let good_ps = good - good_prev;
                let err_svc_er_ps = err_svc_er - err_svc_er_prev;
                let err_cl_miss_ps = err_cl_miss - err_cl_miss_prev;
                let err_cl_to_ps = err_cl_to - err_cl_to_prev;
                log::warn!(
                    "secs: {}, rps: {}, good: {}, err_svc_er: {} err_cl_miss: {}, err_cl_to: {}",
                    secs,
                    rps,
                    good_ps,
                    err_svc_er_ps,
                    err_cl_miss_ps,
                    err_cl_to_ps,
                );
                log::warn!(
                    "secs: {}, err_svc_er_sum: {}, err_cl_miss_sum: {}, err_cl_to_sum: {}, err_search: {}, err_reserve: {}",
                    secs,
                    err_svc_er,
                    err_cl_miss,
                    err_cl_to,
                    err_search,
                    err_reserve,
                );
                all_prev = all;
                good_prev = good;
                err_svc_er_prev = err_svc_er;
                err_cl_miss_prev = err_cl_miss;
                err_cl_to_prev = err_cl_to;

                if Instant::now() > pause_at {
                    break;
                }
            }
        });

        let mut counter_test_id = 0;
        let mut elapse = 0f64;
        let exponential = Exp::new(self.rps as f64).unwrap();
        let uniform = Uniform::<u32>::new(0, 1_000_000_007);

        loop {
            if Instant::now() > pause_at {
                break;
            }

            let start_at = init_at + Duration::from_secs_f64(elapse);
            tokio::time::sleep_until(start_at).await;

            let value = {
                if Instant::now() < warm_at {
                    0.01
                } else {
                    if self.gen_cfg.gap == "const" {
                        1f64 / self.rps as f64
                    } else {
                        exponential.sample(&mut self.rng)
                    }
                }
            };
            elapse += value;

            let api_idx = uniform.sample(&mut self.rng) as usize % self.gen_cfg.apis.len();
            let api = self.gen_cfg.apis[api_idx].clone();
            let slo = self.gen_cfg.slos[api_idx];

            let ctx = {
                let test_id = counter_test_id;
                counter_test_id += 1;
                let request_id = uniform.sample(&mut self.rng) as u64;
                let request_class = 0;

                let start_at = time_now();
                let deadline = start_at + slo;
                let latest_exec = deadline;

                Context::new(
                    api.clone(),
                    test_id,
                    request_id,
                    slo,
                    request_class,
                    start_at,
                    deadline,
                    latest_exec,
                )
            };

            let mut client = self.client.clone();
            let trace_tx = self.trace_tx.clone();
            let all = cnt_all.clone();
            let good = cnt_good.clone();
            let err_svc_er = cnt_err_svc_er.clone();
            let err_cl_miss = cnt_err_cl_miss.clone();
            let err_cl_to = cnt_err_cl_to.clone();

            if api == "Search" {
                let request = {
                    let mut request = tonic::Request::new(get_search_request());
                    request.metadata_mut().insert_ctx("ctx", &ctx);
                    request
                };

                let err_search = cnt_err_search.clone();
                tokio::task::spawn(async move {
                    let send_at = time_now();
                    let timeout_duration = Duration::from_secs(1);
                    let response = timeout(timeout_duration, client.handle_search(request)).await;
                    match response {
                        Ok(response) => {
                            if Instant::now() > trace_at {
                                let recv_at = time_now();
                                let latency = recv_at - send_at;
                                let error = {
                                    if let Err(ref status) = response {
                                        status.message().to_string()
                                    } else if latency > ctx.slo() {
                                        "/LGMiss".to_string()
                                    } else {
                                        "/None".to_string()
                                    }
                                };
                                if error.contains("None") {
                                    good.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    err_search.fetch_add(1, Ordering::Relaxed);
                                    if error.contains("LGMiss") {
                                        err_cl_miss.fetch_add(1, Ordering::Relaxed);
                                    } else if error.contains("EarlyReturn") {
                                        err_svc_er.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        panic!("Unimplemented error: {}", error);
                                    }
                                }
                                let span = Span::new(ctx, latency, error);
                                trace_tx.try_send(span).unwrap();
                            }
                        }
                        Err(_) => {
                            if Instant::now() > trace_at {
                                err_search.fetch_add(1, Ordering::Relaxed);
                                err_cl_to.fetch_add(1, Ordering::Relaxed);
                                let error = "/LGTimeout".to_string();
                                let span = Span::new(ctx, 0, error);
                                trace_tx.try_send(span).unwrap();
                            }
                        }
                    }
                });
            } else if api == "Reservation" {
                let request = {
                    let mut request = tonic::Request::new(get_reservation_request());
                    request.metadata_mut().insert_ctx("ctx", &ctx);
                    request
                };

                let err_reservation = cnt_err_reservation.clone();
                tokio::task::spawn(async move {
                    let send_at = time_now();
                    let timeout_duration = Duration::from_secs(1);
                    let response =
                        timeout(timeout_duration, client.handle_reservation(request)).await;
                    match response {
                        Ok(response) => {
                            if Instant::now() > trace_at {
                                let recv_at = time_now();
                                let latency = recv_at - send_at;
                                let error = {
                                    if let Err(ref status) = response {
                                        status.message().to_string()
                                    } else if latency > ctx.slo() {
                                        "/LGMiss".to_string()
                                    } else {
                                        "/None".to_string()
                                    }
                                };
                                if error.contains("None") {
                                    good.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    err_reservation.fetch_add(1, Ordering::Relaxed);
                                    if error.contains("LGMiss") {
                                        err_cl_miss.fetch_add(1, Ordering::Relaxed);
                                    } else if error.contains("EarlyReturn") {
                                        err_svc_er.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        panic!("Unimplemented error: {}", error);
                                    }
                                }
                                let span = Span::new(ctx, latency, error);
                                trace_tx.try_send(span).unwrap();
                            }
                        }
                        Err(_) => {
                            if Instant::now() > trace_at {
                                err_reservation.fetch_add(1, Ordering::Relaxed);
                                err_cl_to.fetch_add(1, Ordering::Relaxed);
                                let error = "/LGTimeout".to_string();
                                let span = Span::new(ctx, 0, error);
                                trace_tx.try_send(span).unwrap();
                            }
                        }
                    }
                });
            } else {
                panic!("Unimplemented API");
            }
            all.fetch_add(1, Ordering::Relaxed);
        }

        tokio::time::sleep(Duration::from_secs(3)).await;
        log::warn!("Load generated");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    let hotel_cfg: HotelConfig = {
        let file = File::open(args.hotel_config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };
    log::warn!("Hotel config: {:?}", hotel_cfg);
    let gen_cfg: GenConfig = {
        let file = File::open(args.gen_config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };
    assert!(gen_cfg.gap == "const" || gen_cfg.gap == "exp");
    log::warn!("Gen config: {:?}", gen_cfg);

    for rps in &gen_cfg.rps_values {
        log::warn!("Running rps: {}...", rps);

        let output = format!("{}/r{}_{}.csv", args.output_path, rps, args.run_idx);
        let (trace_tx, trace_rx) = unbounded();

        let mut load_gen = {
            const KEY: u64 = 13;
            const SEED: u64 = 998244353;

            let seed = SEED * KEY + rps;
            let rng = StdRng::seed_from_u64(seed);
            let client = {
                let mut client = FrontendClient::connect(gen_cfg.addr.clone()).await?;
                let mut request = tonic::Request::new(get_ping_request());
                let ctx = {
                    let test_id = 0;
                    let request_id = 0;
                    let slo = 1_000_000;
                    let request_class = 0;
                    let start_at = time_now();
                    let deadline = start_at + slo;
                    let latest_exec = deadline;
                    Context::new(
                        "Ping".to_string(),
                        test_id,
                        request_id,
                        slo,
                        request_class,
                        start_at,
                        deadline,
                        latest_exec,
                    )
                };
                request.metadata_mut().insert_ctx("ctx", &ctx);
                client.handle_ping(request).await?;
                client
            };

            let load_gen = LoadGenerator::new(
                hotel_cfg.clone(),
                gen_cfg.clone(),
                rng,
                *rps,
                client,
                trace_tx,
            );
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
