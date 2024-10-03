pub mod bridge {
    tonic::include_proto!("bridge");
}
mod common;
mod graph;

use std::sync::atomic::AtomicUsize;
use std::sync::{
    atomic::{AtomicI32, Ordering},
    Arc,
};

use crossbeam_channel::{unbounded, Sender};
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp, Uniform};
use structopt::StructOpt;
use tokio::time::{Duration, Instant};

use tonic::transport::Channel;
use tonic_masa::{Context, GlobalGraph, FIFO, FIFO_TWO, PRIO_GLOBAL, PRIO_LOCAL};

use bridge::{worker_client::WorkerClient, HelloRequest};
use common::{fetch_traces, get_global_graphs, init_logging, time_now, Span};

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Reboot for simulation")]
pub struct Args {
    #[structopt(long, required = true)]
    pub graph_ids: Vec<String>,
    #[structopt(long, required = true)]
    pub slos: Vec<u64>,
    #[structopt(long, required = true)]
    pub rps: u64,
    #[structopt(long, required = true)]
    pub secs: u64,
    #[structopt(long, required = true)]
    pub concurrency: u64,
    #[structopt(long, required = true)]
    pub output: String,
    #[structopt(long, required = true)]
    pub addr: String,
}

#[derive(Debug)]
struct LoadGenerator {
    rng: StdRng,
    rps: u64,
    secs: u64,
    token: Arc<AtomicI32>,
    global_graphs: Vec<GlobalGraph>,
    client: WorkerClient<Channel>,
    trace_tx: Sender<Span>,
}

impl LoadGenerator {
    fn new(
        rng: StdRng,
        rps: u64,
        secs: u64,
        token: Arc<AtomicI32>,
        global_graphs: Vec<GlobalGraph>,
        client: WorkerClient<Channel>,
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
        LoadGenerator {
            rng,
            rps,
            secs,
            token,
            global_graphs,
            client,
            trace_tx,
        }
    }

    async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
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

        let request = HelloRequest {
            name: "Tonic".into(),
        };

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
            let request_class = request_id as usize % self.global_graphs.len();
            let graph_id = self.global_graphs[request_class].graph_id().clone();
            let slo = self.global_graphs[request_class].slo();

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
                // [TODO] This is a hack for client bench.
                let latest_exec_at = deadline;
                let request_class = request_class as u64;
                let ctx = Context::new(
                    graph_id.clone(),
                    request_id,
                    deadline,
                    latest_exec_at,
                    request_class,
                );
                let mut request = tonic::Request::new(request.clone());
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
                    if graph_id == "I1" {
                        client.say_hello_i1(request).await.unwrap();
                    } else if graph_id.contains("I2") {
                        client.say_hello_i2(request).await.unwrap();
                    } else if graph_id.contains("I4") {
                        client.say_hello_i4(request).await.unwrap();
                    } else {
                        panic!("Unsupported graph_id: {}", graph_id);
                    }
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
    assert!(args.graph_ids.len() == args.slos.len());

    let (trace_tx, trace_rx) = unbounded();

    let mut load_gen = {
        const KEY: u64 = 13;
        const SEED: u64 = 998244353;
        let seed = SEED * KEY + args.rps;
        let rng = StdRng::seed_from_u64(seed);
        let token = Arc::new(AtomicI32::new(args.concurrency as i32));
        let global_graphs = get_global_graphs(&args.graph_ids, &args.slos);
        let client = WorkerClient::connect(args.addr).await?;

        let load_gen = LoadGenerator::new(
            rng,
            args.rps,
            args.secs,
            token,
            global_graphs,
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
        fetch_traces(args.output, trace_rx).await;
    }));

    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}
