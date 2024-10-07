pub mod hotel {
    tonic::include_proto!("frontend");
}

use std::error::Error;
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
use tonic_masa::{Context, GraphId};

use reboot_hotel::{fetch_traces, init_logging, time_now, Span};

use hotel::{frontend_client::FrontendClient, SearchRequest};

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
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
        rng: StdRng,
        graph_id: GraphId,
        rps: u64,
        secs: u64,
        token: Arc<AtomicUsize>,
        client: FrontendClient<Channel>,
        trace_tx: Sender<Span>,
    ) -> Self {
        Self {
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
            let graph_id = self.graph_id.clone();
            let slo = 10_000;

            let request = {
                let search_request = SearchRequest { ave: 61 };
                let mut request = tonic::Request::new(search_request);

                let deadline = time_now() - init_at_u64 + slo;
                let latest_exec_at = deadline;
                let request_class = 0;
                let ctx = Context::new(
                    graph_id.clone(),
                    request_id,
                    deadline,
                    latest_exec_at,
                    request_class,
                );
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

    let (trace_tx, trace_rx) = unbounded();

    let mut load_gen = {
        const KEY: u64 = 13;
        const SEED: u64 = 998244353;

        let graph_id: GraphId = "Hotel".to_string();
        let seed = SEED * KEY + args.rps;
        let rng = StdRng::seed_from_u64(seed);
        let token = Arc::new(AtomicUsize::new(args.concurrency));
        let client = FrontendClient::connect(args.addr).await?;

        let load_gen =
            LoadGenerator::new(rng, graph_id, args.rps, args.secs, token, client, trace_tx);
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
