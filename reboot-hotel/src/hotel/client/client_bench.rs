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
use tokio::time::{Duration, Instant};

use tonic::transport::Channel;
use tonic_masa::{Context, GraphId, FIFO, FIFO_TWO, PRIO_GLOBAL, PRIO_LOCAL};

use config::Config;
use hotel::{frontend_client::FrontendClient, SearchRequest};
use reboot_hotel::{fetch_traces, init_logging, time_now, Span};

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
    #[structopt(long, required = true)]
    pub rps: u64,
    #[structopt(long, required = true)]
    pub secs: u64,
    #[structopt(long, required = true)]
    pub concurrency: usize,
    #[structopt(long, required = true)]
    pub output: String,
    #[structopt(long, default_value = "http://[::1]:8660")]
    pub addr: String,
}

#[derive(Debug)]
struct LoadGenerator {
    cfg: Config,
    rng: StdRng,
    graph_id: GraphId,
    rps: u64,
    secs: u64,
    token: Arc<AtomicUsize>,
    client: FrontendClient<Channel>,
    trace_tx: Sender<Span>,
}

impl LoadGenerator {
    pub fn new(
        cfg: Config,
        rng: StdRng,
        graph_id: GraphId,
        rps: u64,
        secs: u64,
        token: Arc<AtomicUsize>,
        client: FrontendClient<Channel>,
        trace_tx: Sender<Span>,
    ) -> Self {
        if cfg!(feature = "prio_class") {
            log::warn!("Enabled prio_class");
        } else if cfg!(feature = "prio_global") {
            log::warn!("Enabled prio_global");
        } else if cfg!(feature = "prio_class_global") {
            log::warn!("Enabled prio_class_global");
        } else if cfg!(feature = "prio_local") {
            log::warn!("Enabled prio_local");
        } else if cfg!(feature = "fifo_two") {
            log::warn!("Enabled fifo_two");
        } else if cfg!(feature = "fifo") {
            log::warn!("Enabled fifo");
        } else {
            panic!("Not implemented policy");
        }
        Self {
            cfg,
            rng,
            graph_id,
            rps,
            secs,
            token,
            client,
            trace_tx,
        }
    }

    async fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        tokio::task::spawn(async move {
            let mut counter_before = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let counter_now = counter_clone.load(Ordering::Relaxed);
                log::info!("RPS: {}", counter_now - counter_before);
                counter_before = counter_now;
            }
        });

        let init_at = Instant::now();
        let init_at_u64 = time_now();
        let pause_at = init_at + Duration::from_secs(self.secs);

        let mut elapse = 0f64;
        let exponential = Exp::new(self.rps as f64).unwrap();
        let uniform = Uniform::new(0, 1_000_000_007);

        loop {
            if Instant::now() > pause_at {
                break;
            }

            if self.token.load(Ordering::SeqCst) <= 0 {
                continue;
            }

            let start_at = init_at + Duration::from_secs_f64(elapse);
            tokio::time::sleep_until(start_at).await;

            let value = exponential.sample(&mut self.rng);
            // let value = 1f64 / self.rps as f64;
            elapse += value;

            let request_id = uniform.sample(&mut self.rng) as u64;
            let request_class = 0;
            let graph_id = self.graph_id.clone();
            let slo = 10_000;

            let request = {
                let deadline = {
                    if PRIO_GLOBAL || PRIO_LOCAL {
                        let start_at = time_now() - init_at_u64;
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
                    deadline,
                    latest_exec_at,
                    request_class,
                );

                let ave = (request_id % self.cfg.hotels as u64) as u32;
                let search_request = SearchRequest { ave };
                let mut request = tonic::Request::new(search_request);
                request.metadata_mut().insert_ctx("ctx", &ctx);

                request
            };

            if self.token.load(Ordering::SeqCst) > 0 {
                counter.fetch_add(1, Ordering::Relaxed);
                let token = self.token.clone();
                token.fetch_sub(1, Ordering::SeqCst);

                let mut client = self.client.clone();
                let trace_tx = self.trace_tx.clone();

                tokio::task::spawn(async move {
                    let send_at = time_now();
                    client.handle_search(request).await.unwrap();
                    let recv_at = time_now();
                    let latency = recv_at - send_at;
                    let span = Span::new(request_id, graph_id, slo, latency);
                    token.fetch_add(1, Ordering::SeqCst);
                    trace_tx.try_send(span).unwrap();
                });
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    let file = File::open(args.config).expect("Failed to open file");
    let reader = BufReader::new(file);
    let cfg: Config = serde_json::from_reader(reader)?;
    log::info!("Hotel config: {:?}", cfg);

    let (trace_tx, trace_rx) = unbounded();

    let mut load_gen = {
        const KEY: u64 = 13;
        const SEED: u64 = 998244353;

        let graph_id: GraphId = "Hotel".to_string();
        let seed = SEED * KEY + args.rps;
        let rng = StdRng::seed_from_u64(seed);
        let token = Arc::new(AtomicUsize::new(args.concurrency));
        let client = FrontendClient::connect(args.addr).await?;

        let load_gen = LoadGenerator::new(
            cfg, rng, graph_id, args.rps, args.secs, token, client, trace_tx,
        );
        load_gen
    };

    let mut handles = Vec::new();

    handles.push(tokio::spawn(async move {
        load_gen.run().await.unwrap();
    }));

    handles.push(tokio::spawn(async move {
        fetch_traces(args.output, trace_rx).await;
    }));

    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}
