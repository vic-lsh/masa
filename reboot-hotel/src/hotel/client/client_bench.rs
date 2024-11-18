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
use gen::{gen_reserve_request, gen_search_request};
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp, Uniform};
use structopt::StructOpt;
use tokio::time::{timeout, Duration, Instant};

use tonic::transport::Channel;
use tonic_masa::{
    Context, FIFO, FIFO_INFRA, PRIO_GLOBAL, PRIO_GLOBAL_EARLY, PRIO_LOCAL, PRIO_LOCAL_EARLY,
};

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
        let cnt_success = Arc::new(AtomicUsize::new(0));
        let cnt_err_svc = Arc::new(AtomicUsize::new(0));
        let cnt_err_client = Arc::new(AtomicUsize::new(0));
        let cnt_err_client_ot = Arc::new(AtomicUsize::new(0));

        let cnt_err_search = Arc::new(AtomicUsize::new(0));
        let cnt_err_reserve = Arc::new(AtomicUsize::new(0));

        let cnt_all_clone = cnt_all.clone();
        let cnt_success_clone = cnt_success.clone();
        let cnt_err_svc_clone = cnt_err_svc.clone();
        let cnt_err_client_clone = cnt_err_client.clone();
        let cnt_err_client_ot_clone = cnt_err_client_ot.clone();
        let cnt_err_search_clone = cnt_err_search.clone();
        let cnt_err_reserve_clone = cnt_err_reserve.clone();

        tokio::task::spawn(async move {
            let mut all_prev = 0;
            let mut succ_prev = 0;
            let mut err_svc_prev = 0;
            let mut err_client_prev = 0;
            let mut err_client_to_prev = 0;
            let mut secs = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let all = cnt_all_clone.load(Ordering::Relaxed);
                let succ = cnt_success_clone.load(Ordering::Relaxed);
                let err_svc = cnt_err_svc_clone.load(Ordering::Relaxed);
                let err_client = cnt_err_client_clone.load(Ordering::Relaxed);
                let err_client_ot = cnt_err_client_ot_clone.load(Ordering::Relaxed);
                let err_search = cnt_err_search_clone.load(Ordering::Relaxed);
                let err_reserve = cnt_err_reserve_clone.load(Ordering::Relaxed);

                secs += 1;

                let rps = all - all_prev;
                let good_ps = succ - succ_prev;
                let err_svc_ps = err_svc - err_svc_prev;
                let err_cl_ps = err_client - err_client_prev;
                let err_cl_ot_ps = err_client_ot - err_client_to_prev;
                log::warn!(
                    "secs: {}, rps: {}, good: {}, err_svc: {} err_cl: {}, err_cl_ot: {}",
                    secs,
                    rps,
                    good_ps,
                    err_svc_ps,
                    err_cl_ps,
                    err_cl_ot_ps,
                );
                log::warn!(
                    "secs: {}, err_svc_sum: {}, err_cl_sum: {}, err_cl_ot_sum: {}, err_search: {}, err_reserve: {}",
                    secs,
                    err_svc,
                    err_client,
                    err_client_ot,
                    err_search,
                    err_reserve,
                );
                all_prev = all;
                succ_prev = succ;
                err_svc_prev = err_svc;
                err_client_prev = err_client;
                err_client_to_prev = err_client_ot;

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
                let deadline = {
                    if PRIO_GLOBAL || PRIO_GLOBAL_EARLY || PRIO_LOCAL || PRIO_LOCAL_EARLY {
                        // [DEPRECATED] Relative start time.
                        // let start_at = time_now() - init_at_u64;
                        start_at + slo
                    } else if FIFO_INFRA || FIFO {
                        slo
                    } else {
                        panic!("Unimplemented policy")
                    }
                };
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
            let good = cnt_success.clone();
            let err_svc = cnt_err_svc.clone();
            let err_client = cnt_err_client.clone();
            let err_client_ot = cnt_err_client_ot.clone();

            if api == "Search" {
                let request = {
                    let mut request = tonic::Request::new(gen_search_request());
                    request.metadata_mut().insert_ctx("ctx", &ctx);
                    request
                };

                let err_search = cnt_err_search.clone();
                tokio::task::spawn(async move {
                    let send_at = time_now();
                    let timeout_duration = Duration::from_secs(30);
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
                                if error == "/None" {
                                    good.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    if error == "/LGMiss" {
                                        err_client.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        err_svc.fetch_add(1, Ordering::Relaxed);
                                    }
                                    err_search.fetch_add(1, Ordering::Relaxed);
                                }
                                let span = Span::new(ctx, latency, error);
                                trace_tx.try_send(span).unwrap();
                            }
                        }
                        Err(_) => {
                            if Instant::now() > trace_at {
                                err_search.fetch_add(1, Ordering::Relaxed);
                                err_client_ot.fetch_add(1, Ordering::Relaxed);
                                let error = "/LGTimeout".to_string();
                                let span = Span::new(ctx, 0, error);
                                trace_tx.try_send(span).unwrap();
                            }
                        }
                    }
                });
            } else if api == "Reservation" {
                let request = {
                    let mut request = tonic::Request::new(gen_reserve_request());
                    request.metadata_mut().insert_ctx("ctx", &ctx);
                    request
                };

                let err_reserve = cnt_err_reserve.clone();
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
                                if error == "/None" {
                                    good.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    if error == "/LGMiss" {
                                        err_client.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        err_svc.fetch_add(1, Ordering::Relaxed);
                                    }
                                    err_reserve.fetch_add(1, Ordering::Relaxed);
                                }
                                let span = Span::new(ctx, latency, error);
                                trace_tx.try_send(span).unwrap();
                            }
                        }
                        Err(_) => {
                            if Instant::now() > trace_at {
                                err_reserve.fetch_add(1, Ordering::Relaxed);
                                err_client_ot.fetch_add(1, Ordering::Relaxed);
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
            let client = FrontendClient::connect(gen_cfg.addr.clone()).await?;

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
