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
use tonic_masa::{
    Context, GraphId, FIFO, FIFO_TWO, PRIO_GLOBAL, PRIO_GLOBAL_TWO, PRIO_LOCAL, PRIO_LOCAL_TWO,
};

use config::{GenConfig, HotelConfig};
use hotel::{frontend_client::FrontendClient, SearchRequest};
use reboot_hotel::{fetch_traces, init_logging, time_now, Span};

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub hotel_config: PathBuf,
    #[structopt(short, long, required = true)]
    pub gen_config: PathBuf,
}

// Randomly generate a new search request.
//
// [NOTE] this is a port of the original search request generation logic:
// https://github.com/delimitrou/DeathStarBench/blob/6ecb09706140f8730b5385c08f1386c654c3c526/hotelReservation/wrk2/scripts/hotel-reservation/mixed-workload_type_1.lua#L17
fn gen_search_request() -> SearchRequest {
    use rand::Rng;

    let mut rng = rand::thread_rng();

    // Generate random dates
    let in_date_day_num: i32 = rng.gen_range(9..=23);
    let out_date_day_num: i32 = rng.gen_range((in_date_day_num + 1)..=24);

    // Format in_date string
    let in_date = if in_date_day_num <= 9 {
        format!("2015-04-0{}", in_date_day_num)
    } else {
        format!("2015-04-{}", in_date_day_num)
    };

    // Format out_date string
    let out_date = if out_date_day_num <= 9 {
        format!("2015-04-0{}", out_date_day_num)
    } else {
        format!("2015-04-{}", out_date_day_num)
    };

    // Generate random coordinates
    let lat = 38.0235 + (rng.gen_range(0..=481) as f64 - 240.5) / 1000.0;
    let lon = -122.095 + (rng.gen_range(0..=325) as f64 - 157.0) / 1000.0;

    SearchRequest {
        lat,
        lon,
        in_date,
        out_date,
        locale: None,
    }
}

#[derive(Debug)]
struct LoadGenerator {
    _hotel_cfg: HotelConfig,
    gen_cfg: GenConfig,
    rng: StdRng,
    graph_id: GraphId,
    token: Arc<AtomicUsize>,
    client: FrontendClient<Channel>,
    trace_tx: Sender<Span>,
}

impl LoadGenerator {
    pub fn new(
        _hotel_cfg: HotelConfig,
        gen_cfg: GenConfig,
        rng: StdRng,
        graph_id: GraphId,
        token: Arc<AtomicUsize>,
        client: FrontendClient<Channel>,
        trace_tx: Sender<Span>,
    ) -> Self {
        if cfg!(feature = "prio_class") {
            log::warn!("Enabled prio_class");
        } else if cfg!(feature = "prio_global") {
            log::warn!("Enabled prio_global");
        } else if cfg!(feature = "prio_global_two") {
            log::warn!("Enabled prio_global_two");
        } else if cfg!(feature = "prio_class_global") {
            log::warn!("Enabled prio_class_global");
        } else if cfg!(feature = "prio_local") {
            log::warn!("Enabled prio_local");
        } else if cfg!(feature = "prio_local_two") {
            log::warn!("Enabled prio_local_two");
        } else if cfg!(feature = "fifo_two") {
            log::warn!("Enabled fifo_two");
        } else if cfg!(feature = "fifo") {
            log::warn!("Enabled fifo");
        } else {
            panic!("Not implemented policy");
        }
        Self {
            _hotel_cfg,
            gen_cfg,
            rng,
            graph_id,
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
        let trace_at = init_at + Duration::from_secs(self.gen_cfg.warmup_secs);
        let pause_at =
            init_at + Duration::from_secs(self.gen_cfg.warmup_secs + self.gen_cfg.duration_secs);

        let mut elapse = 0f64;
        let exponential = Exp::new(self.gen_cfg.rps as f64).unwrap();
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
            let slo = self.gen_cfg.slo;

            let request = {
                let deadline = {
                    if PRIO_GLOBAL || PRIO_GLOBAL_TWO || PRIO_LOCAL || PRIO_LOCAL_TWO {
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

                let search_request = gen_search_request();
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
                    if Instant::now() > trace_at {
                        trace_tx.try_send(span).unwrap();
                    }
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
    let hotel_cfg: HotelConfig = {
        let file = File::open(args.hotel_config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };
    log::info!("Hotel config: {:?}", hotel_cfg);
    let gen_cfg: GenConfig = {
        let file = File::open(args.gen_config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };
    log::info!("Gen config: {:?}", gen_cfg);

    for i in 0..gen_cfg.repeats {
        let output_path = gen_cfg.output.clone();
        let output = format!("{}/r{}_{}.csv", output_path.clone(), gen_cfg.rps, i);

        let (trace_tx, trace_rx) = unbounded();

        let mut load_gen = {
            const KEY: u64 = 13;
            const SEED: u64 = 998244353;

            let graph_id: GraphId = "Hotel".to_string();
            let seed = SEED * KEY + gen_cfg.rps;
            let rng = StdRng::seed_from_u64(seed);
            let token = Arc::new(AtomicUsize::new(gen_cfg.concurrency));
            let client = FrontendClient::connect(gen_cfg.addr.clone()).await?;

            let load_gen = LoadGenerator::new(
                hotel_cfg.clone(),
                gen_cfg.clone(),
                rng,
                graph_id,
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

    Ok(())
}
