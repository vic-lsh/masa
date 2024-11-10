#[path = "../config.rs"]
pub mod config;
pub mod hotel {
    tonic::include_proto!("frontend");
}

use std::cmp;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use crossbeam_channel::{unbounded, Sender};
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp, Uniform};
use structopt::StructOpt;
use tokio::time::{timeout, Duration, Instant};

use tonic::transport::Channel;
use tonic_masa::{
    Context, GraphId, FIFO, FIFO_TWO, PRIO_GLOBAL, PRIO_GLOBAL_EARLY, PRIO_LOCAL, PRIO_LOCAL_EARLY,
};

use config::{GenConfig, HotelConfig};
use hotel::{frontend_client::FrontendClient, ReservationRequest, SearchRequest};
use reboot_hotel::{fetch_traces, init_logging, time_now, Span};

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
    #[structopt(long, required = true)]
    pub hotel_config: PathBuf,
    #[structopt(long, required = true)]
    pub gen_config: PathBuf,
    #[structopt(long, required = true)]
    pub run_idx: String,
}

#[derive(Debug)]
struct LoadGenerator {
    hotel_cfg: HotelConfig,
    gen_cfg: GenConfig,
    rng: StdRng,
    graph_id: GraphId,
    api: String,
    rps: u64,
    client: FrontendClient<Channel>,
    trace_tx: Sender<Span>,
}

impl LoadGenerator {
    pub fn new(
        hotel_cfg: HotelConfig,
        gen_cfg: GenConfig,
        rng: StdRng,
        graph_id: GraphId,
        api: String,
        rps: u64,
        client: FrontendClient<Channel>,
        trace_tx: Sender<Span>,
    ) -> Self {
        if cfg!(feature = "prio_class") {
            log::warn!("Enabled prio_class");
        } else if cfg!(feature = "prio_global") {
            log::warn!("Enabled prio_global");
        } else if cfg!(feature = "prio_global_early") {
            log::warn!("Enabled prio_global_early");
        } else if cfg!(feature = "prio_class_global") {
            log::warn!("Enabled prio_class_global");
        } else if cfg!(feature = "prio_local") {
            log::warn!("Enabled prio_local");
        } else if cfg!(feature = "prio_local_early") {
            log::warn!("Enabled prio_local_early");
        } else if cfg!(feature = "fifo_two") {
            log::warn!("Enabled fifo_two");
        } else if cfg!(feature = "fifo") {
            log::warn!("Enabled fifo");
        } else {
            panic!("Not implemented policy");
        }
        Self {
            hotel_cfg,
            gen_cfg,
            rng,
            graph_id,
            api,
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

        let cnt_all_clone = cnt_all.clone();
        let cnt_success_clone = cnt_success.clone();
        let cnt_err_svc_clone = cnt_err_svc.clone();
        let cnt_err_client_clone = cnt_err_client.clone();
        let cnt_err_client_ot_clone = cnt_err_client_ot.clone();

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
                    "secs: {}, err_svc_sum: {}, err_cl_sum: {}, err_cl_ot_sum: {}",
                    secs,
                    err_svc,
                    err_client,
                    err_client_ot,
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
        // let exponential = Exp::new(self.rps as f64).unwrap();
        let uniform = Uniform::new(0, 1_000_000_007);

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
                    // exponential.sample(&mut self.rng)
                    1f64 / self.rps as f64
                }
            };
            elapse += value;

            let ctx = {
                let test_id = counter_test_id;
                counter_test_id += 1;
                let request_id = uniform.sample(&mut self.rng) as u64;
                let request_class = 0;
                let graph_id = self.graph_id.clone();
                let slo = self.gen_cfg.slo;

                let start_at = time_now();
                let deadline = {
                    if PRIO_GLOBAL || PRIO_GLOBAL_EARLY || PRIO_LOCAL || PRIO_LOCAL_EARLY {
                        // [DEPRECATED] Relative start time.
                        // let start_at = time_now() - init_at_u64;
                        // start_at + slo
                        start_at + slo
                    } else if FIFO_TWO || FIFO {
                        slo
                    } else {
                        panic!("Unimplemented policy")
                    }
                };
                let latest_exec_at = deadline;

                Context::new(
                    graph_id.clone(),
                    test_id,
                    request_id,
                    slo,
                    request_class,
                    start_at,
                    deadline,
                    latest_exec_at,
                )
            };

            let mut client = self.client.clone();
            let trace_tx = self.trace_tx.clone();
            let all = cnt_all.clone();
            let good = cnt_success.clone();
            let err_svc = cnt_err_svc.clone();
            let err_client = cnt_err_client.clone();
            let err_client_ot = cnt_err_client_ot.clone();

            if self.api == "Search" {
                let request = {
                    let customer = "Customer".to_string();
                    let ave = (ctx.request_id() % self.hotel_cfg.hotels as u64) as u32;
                    let dates = {
                        let in_date =
                            uniform.sample(&mut self.rng) % self.hotel_cfg.reservation_dates as u32;
                        let out_date =
                            uniform.sample(&mut self.rng) % self.hotel_cfg.reservation_dates as u32;
                        if in_date < out_date {
                            (in_date, out_date + 1)
                        } else {
                            (out_date, in_date + 1)
                        }
                    };
                    let search_request = SearchRequest {
                        customer,
                        ave,
                        in_date: dates.0,
                        out_date: dates.1,
                    };
                    let mut request = tonic::Request::new(search_request);
                    request.metadata_mut().insert_ctx("ctx", &ctx);
                    request
                };

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
                                if error == "/None" {
                                    good.fetch_add(1, Ordering::Relaxed);
                                } else if error == "/LGMiss" {
                                    err_client.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    err_svc.fetch_add(1, Ordering::Relaxed);
                                }
                                let span = Span::new(ctx, latency, error);
                                trace_tx.try_send(span).unwrap();
                            }
                        }
                        Err(_) => {
                            if Instant::now() > trace_at {
                                err_client_ot.fetch_add(1, Ordering::Relaxed);
                                let error = "/LGTimeout".to_string();
                                let span = Span::new(ctx, 0, error);
                                trace_tx.try_send(span).unwrap();
                            }
                        }
                    }
                });
            } else if self.api == "Reservation" {
                let request = {
                    let accounts = {
                        let id = uniform.sample(&mut self.rng) % self.hotel_cfg.user_users as u32;
                        let username = format!("Username_{}", id);
                        let password = format!("Password_{}", id);
                        (username, password)
                    };
                    let hotels = {
                        let lhs = uniform.sample(&mut self.rng) % self.hotel_cfg.hotels as u32;
                        let rhs = cmp::min(
                            lhs + uniform.sample(&mut self.rng)
                                % self.hotel_cfg.reservation_hotels as u32
                                + 1,
                            self.hotel_cfg.hotels,
                        );
                        let mut hotels = Vec::new();
                        for i in lhs..rhs {
                            hotels.push(format!("Sheraton_Ave_{}", i));
                        }
                        hotels
                    };
                    let dates = {
                        let in_date =
                            uniform.sample(&mut self.rng) % self.hotel_cfg.reservation_dates as u32;
                        let out_date =
                            uniform.sample(&mut self.rng) % self.hotel_cfg.reservation_dates as u32;
                        if in_date < out_date {
                            (in_date, out_date + 1)
                        } else {
                            (out_date, in_date + 1)
                        }
                    };
                    let num_rooms = 1;
                    let reservation_request = ReservationRequest {
                        username: accounts.0,
                        password: accounts.1,
                        customer: "Customer".to_string(),
                        hotels,
                        in_date: dates.0,
                        out_date: dates.1,
                        num_rooms,
                    };
                    let mut request = tonic::Request::new(reservation_request);
                    request.metadata_mut().insert_ctx("ctx", &ctx);
                    request
                };

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
                                } else if error == "/LGMiss" {
                                    err_client.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    err_svc.fetch_add(1, Ordering::Relaxed);
                                }
                                let span = Span::new(ctx, latency, error);
                                trace_tx.try_send(span).unwrap();
                            }
                        }
                        Err(_) => {
                            if Instant::now() > trace_at {
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
    log::warn!("Gen config: {:?}", gen_cfg);

    for rps in &gen_cfg.rps_values {
        log::warn!("Running rps: {}...", rps);

        let output_path = gen_cfg.output.clone();
        let output = format!("{}/r{}_{}.csv", output_path, rps, args.run_idx);
        let (trace_tx, trace_rx) = unbounded();

        let mut load_gen = {
            const KEY: u64 = 13;
            const SEED: u64 = 998244353;

            let graph_id: GraphId = "Hotel".to_string();
            // let api = "Search".to_string();
            let api = "Reservation".to_string();
            let seed = SEED * KEY + rps;
            let rng = StdRng::seed_from_u64(seed);
            let client = FrontendClient::connect(gen_cfg.addr.clone()).await?;

            let load_gen = LoadGenerator::new(
                hotel_cfg.clone(),
                gen_cfg.clone(),
                rng,
                graph_id,
                api,
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
