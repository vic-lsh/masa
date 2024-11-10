#[path = "../config.rs"]
pub mod config;
pub mod hotel {
    tonic::include_proto!("frontend");
}

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
use hotel::{frontend_client::FrontendClient, SearchRequest};
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
    rps: u64,
    token: Arc<AtomicUsize>,
    client: FrontendClient<Channel>,
    trace_tx: Sender<Span>,
}

impl LoadGenerator {
    pub fn new(
        hotel_cfg: HotelConfig,
        gen_cfg: GenConfig,
        rng: StdRng,
        graph_id: GraphId,
        rps: u64,
        token: Arc<AtomicUsize>,
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
        } else if cfg!(feature = "fifo_infra") {
            log::warn!("Enabled fifo_infra");
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
            rps,
            token,
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

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        tokio::task::spawn(async move {
            let mut counter_before = 0;
            let mut secs = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let counter_now = counter_clone.load(Ordering::Relaxed);
                secs += 1;
                log::warn!("secs: {}, rps: {}", secs, counter_now - counter_before);
                counter_before = counter_now;
                if Instant::now() > pause_at {
                    break;
                }
            }
        });

        let mut elapse = 0f64;
        let exponential = Exp::new(self.rps as f64).unwrap();
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
                    exponential.sample(&mut self.rng)
                    // 1f64 / self.rps as f64
                }
            };
            elapse += value;

            // Atomically decrement if we still have remaining concurrency.
            if self
                .token
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |token| {
                    if token > 0 {
                        Some(token - 1)
                    } else {
                        None
                    }
                })
                .is_err()
            {
                // The token was set at 0. We have exhausted our concurrency.
                continue;
            }

            let request_id = uniform.sample(&mut self.rng) as u64;
            let request_class = 0;
            let graph_id = self.graph_id.clone();
            let slo = self.gen_cfg.slo;

            let request = {
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
                let ctx = Context::new(
                    graph_id.clone(),
                    request_id,
                    slo,
                    request_class,
                    start_at,
                    deadline,
                    latest_exec_at,
                );

                let ave = (request_id % self.hotel_cfg.hotels as u64) as u32;
                let search_request = SearchRequest { ave };
                let mut request = tonic::Request::new(search_request);
                request.metadata_mut().insert_ctx("ctx", &ctx);

                request
            };

            // [TODO] Fix token.
            // let token = self.token.clone();

            counter.fetch_add(1, Ordering::Relaxed);
            let mut client = self.client.clone();
            let trace_tx = self.trace_tx.clone();

            tokio::task::spawn(async move {
                let send_at = time_now();
                let timeout_duration = Duration::from_secs(1);
                match timeout(timeout_duration, client.handle_search(request)).await {
                    Ok(response) => {
                        if Instant::now() > trace_at {
                            let recv_at = time_now();
                            let latency = recv_at - send_at;
                            let fe_latency = {
                                if let Some(response) = response.as_ref().ok() {
                                    let ctx = response.metadata().get_ctx("ctx").unwrap();
                                    ctx.frontend_elapse().unwrap()
                                } else {
                                    0
                                }
                            };
                            let error = {
                                if let Err(ref status) = response {
                                    status.message().to_string()
                                } else {
                                    "/None".to_string()
                                }
                            };
                            let span =
                                Span::new(request_id, graph_id, slo, latency, fe_latency, error);
                            trace_tx.try_send(span).unwrap();
                        }
                    }
                    Err(_) => {
                        if Instant::now() > trace_at {
                            let error = "/LoadGen".to_string();
                            let span = Span::new(request_id, graph_id, slo, 0, 0, error);
                            trace_tx.try_send(span).unwrap();
                        }
                    }
                }
            });
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
        log::warn!("Running rps: {}", rps);

        let output_path = gen_cfg.output.clone();
        let output = format!("{}/r{}_{}.csv", output_path, rps, args.run_idx);
        let (trace_tx, trace_rx) = unbounded();

        let mut load_gen = {
            const KEY: u64 = 13;
            const SEED: u64 = 998244353;

            let graph_id: GraphId = "Hotel".to_string();
            let seed = SEED * KEY + rps;
            let rng = StdRng::seed_from_u64(seed);
            let token = Arc::new(AtomicUsize::new(gen_cfg.concurrency));
            let client = FrontendClient::connect(gen_cfg.addr.clone()).await?;

            let load_gen = LoadGenerator::new(
                hotel_cfg.clone(),
                gen_cfg.clone(),
                rng,
                graph_id,
                *rps,
                token,
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
