use bridge::{worker_client::WorkerClient, HelloRequest};
use crossbeam_channel::{unbounded, Receiver, Sender};
use env_logger::{Builder, Env};
use log::info;
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp, Uniform};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::{
    atomic::{AtomicI32, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
use tokio::time::{Duration, Instant};
use tonic::transport::Channel;
use tonic_masa::{Context, GlobalGraph};

pub mod bridge {
    tonic::include_proto!("bridge");
}
mod graph;

pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Reboot for simulation")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub slo: u64,
    #[structopt(short, long, required = true)]
    pub rps: u64,
    #[structopt(short, long, required = true)]
    pub secs: u64,
    #[structopt(short, long, required = true)]
    pub concurrency: u64,
    #[structopt(short, long, required = true)]
    pub output: String,
    #[structopt(short, long, required = true)]
    pub graph_id: String,
    #[structopt(short, long, required = true)]
    pub addr: String,
}

#[derive(Debug, Clone)]
pub struct Span {
    request_id: u64,
    span: String,
    slo: u64,
    latency: u64,
}

impl Span {
    pub fn new(request_id: u64, span: String, slo: u64, latency: u64) -> Self {
        Self {
            request_id,
            span,
            slo,
            latency,
        }
    }
}

async fn fetch_traces(output: String, trace_rx: Receiver<Span>) {
    let path = Path::new(&output);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).unwrap();
        }
    }
    let mut file = File::create(output).unwrap();
    writeln!(file, "request_id,span,slo,latency").unwrap();
    while let Ok(span) = trace_rx.recv() {
        writeln!(
            file,
            "{},{},{},{}",
            span.request_id, span.span, span.slo, span.latency
        )
        .unwrap();
    }
}

#[derive(Debug)]
struct LoadGenerator {
    rng: StdRng,
    slo: u64,
    rps: u64,
    secs: u64,
    token: Arc<AtomicI32>,
    global_graph: GlobalGraph,
    client: WorkerClient<Channel>,
    trace_tx: Sender<Span>,
}

impl LoadGenerator {
    fn new(
        rng: StdRng,
        slo: u64,
        rps: u64,
        secs: u64,
        token: Arc<AtomicI32>,
        global_graph: GlobalGraph,
        client: WorkerClient<Channel>,
        trace_tx: Sender<Span>,
    ) -> Self {
        LoadGenerator {
            rng,
            slo,
            rps,
            secs,
            token,
            global_graph,
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
                info!("RPS: {}", counter_now - counter_before);
                counter_before = counter_now;
            }
        });

        let init_at = Instant::now();
        let init_at_u64 = time_now();
        let pause_at = init_at + Duration::from_secs(self.secs);

        let mut elapse = 0f64;
        let exponential = Exp::new(self.rps as f64).unwrap();
        let uniform = Uniform::new(0, 1_000_000_007);

        let graph_id = self.global_graph.graph_id().clone();
        let local_graph = self.global_graph.get_source().clone();
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

            let graph_id = graph_id.clone();
            let request_id = uniform.sample(&mut self.rng);
            let request = {
                let start_at = time_now() - init_at_u64;
                let deadline = start_at + self.slo;
                // [TODO] Support Hotel.
                let ctx = Context::new(
                    graph_id.clone(),
                    request_id,
                    start_at,
                    deadline,
                    Some(local_graph.clone()),
                );
                let mut request = tonic::Request::new(request.clone());
                request.metadata_mut().insert_ctx("par_ctx", &ctx);
                request
            };

            if self.token.load(Ordering::SeqCst) > 0 {
                counter.fetch_add(1, Ordering::Relaxed);
                let token = self.token.clone();
                token.fetch_sub(1, Ordering::SeqCst);

                let slo = self.slo;
                let mut client = self.client.clone();
                let trace_tx = self.trace_tx.clone();

                tokio::task::spawn(async move {
                    let send_at = time_now();
                    if graph_id == "I1" {
                        client.say_hello_i1(request).await.unwrap();
                    } else if graph_id == "I2" {
                        client.say_hello_i2(request).await.unwrap();
                    } else if graph_id == "I4" {
                        client.say_hello_i4(request).await.unwrap();
                    } else {
                        panic!("Unsupported graph_id: {}", graph_id);
                    }
                    let recv_at = time_now();
                    let latency = recv_at - send_at;
                    let span_str = {
                        if graph_id == "I1" {
                            "SayHelloI1".to_string()
                        } else if graph_id == "I2" {
                            "SayHelloI2".to_string()
                        } else if graph_id == "I4" {
                            "SayHelloI4".to_string()
                        } else {
                            panic!("Unsupported graph_id: {}", graph_id);
                        }
                    };
                    let span = Span::new(request_id, span_str, slo, latency);
                    token.fetch_add(1, Ordering::SeqCst);
                    trace_tx.try_send(span).unwrap();
                });
            }
        }

        Ok(())
    }
}

fn init_logging() {
    Builder::from_env(Env::default().default_filter_or("info"))
        .format(|buf, record| {
            use std::io::Write;
            writeln!(
                buf,
                "{} [{}:{}] {}",
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                // record.target(),
                record.args()
            )
        })
        .init();
    info!("Logging initialized");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();

    let (trace_tx, trace_rx) = unbounded();

    let mut handles = Vec::new();

    let mut load_gen = {
        const KEY: u64 = 13;
        const SEED: u64 = 998244353;
        let seed = SEED * KEY + args.rps;
        let rng = StdRng::seed_from_u64(seed);
        let token = Arc::new(AtomicI32::new(args.concurrency as i32));
        let global_graph = {
            if args.graph_id == "I1" {
                graph::get_global_graph_i1()
            } else if args.graph_id == "I2" {
                graph::get_global_graph_i2()
            } else if args.graph_id == "I4" {
                graph::get_global_graph_i4()
            } else if args.graph_id == "Hotel" {
                graph::get_global_graph_hotel()
            } else {
                panic!("Unsupported graph_id: {}", args.graph_id);
            }
        };
        assert!(global_graph.graph_id().to_string() == args.graph_id);
        let client = WorkerClient::connect(args.addr).await?;

        let load_gen = LoadGenerator::new(
            rng,
            args.slo,
            args.rps,
            args.secs,
            token,
            global_graph,
            client,
            trace_tx,
        );
        load_gen
    };

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
