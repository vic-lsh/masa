use crossbeam_channel::{bounded, unbounded, Receiver, Sender, TryRecvError};
use masa::{frontend_client::FrontendClient, SearchRequest};
use rand::Rng;
use rand_distr::{Distribution, Exp};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
use tokio::{
    task::JoinSet,
    time::{Duration, Instant},
};
use tonic::Request;

pub mod masa {
    tonic::include_proto!("frontend");
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub rps: u64,
    #[structopt(short, long, required = true)]
    pub secs: u64,
    #[structopt(short, long, required = true)]
    pub concurrency: u32,
    #[structopt(short, long, required = true)]
    pub addr: String,
    #[structopt(short, long, required = true)]
    pub output: String,
}

pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

#[derive(Debug, Clone)]
struct Span {
    send_at: u64,
    recv_at: u64,
}

async fn load_gen(rps: u64, secs: u64, concurrency: u32, addr: String, trace_tx: Sender<Span>) {
    eprintln!("Client connecting to {}...", addr);
    let client = FrontendClient::connect(addr)
        .await
        .expect("Failed to connect to server");

    let start_at = Instant::now();
    let pause_at = start_at + Duration::from_secs(secs);
    let timeout_at = start_at + Duration::from_secs(secs + 3);
    let mut elapse = 0f64;
    let mut elapse_secs = 0u64;

    let total = rps * secs;
    let mut cnt = 0u64;
    let mut cnt_per_sec = 0u64;

    let exponential = Exp::new(rps as f64).expect("Failed to create exponential distribution");
    let mut rng = rand::thread_rng();

    let (request_tx, request_rx) = bounded(concurrency as usize);
    let mut join_set = JoinSet::new();

    for _ in 0..concurrency {
        let mut client = client.clone();
        let trace_tx = trace_tx.clone();
        let request_rx = request_rx.clone();
        join_set.spawn(async move {
            loop {
                match request_rx.try_recv() {
                    Ok(req) => {
                        let send_at = time_now();
                        match client.handle_search(req).await {
                            Ok(_) => {
                                let recv_at = time_now();
                                let span = Span { send_at, recv_at };
                                trace_tx.send(span).expect("Failed to send trace");
                            }
                            Err(_) => {}
                        }
                    }
                    Err(TryRecvError::Disconnected) => {
                        break;
                    }
                    Err(TryRecvError::Empty) => {
                        tokio::task::yield_now().await;
                    }
                }
            }
        });
    }

    loop {
        let now = Instant::now();
        let elapse_at = start_at + Duration::from_secs(elapse_secs);
        if now > elapse_at {
            eprintln!("elapse secs: {}, cnt per sec: {}", elapse_secs, cnt_per_sec);
            elapse_secs += 1;
            cnt_per_sec = 0;
        }
        if now > timeout_at {
            break;
        }

        if now < pause_at && cnt < total {
            let send_at = start_at + Duration::from_secs_f64(elapse);
            tokio::time::sleep_until(send_at).await;
            cnt += 1;
            cnt_per_sec += 1;
            elapse += exponential.sample(&mut rng);
            // [TODO] Ave range.
            let ave = rng.gen_range(0..10_000);
            let request = Request::new(SearchRequest { ave });
            request_tx.send(request).expect("Failed to send request");
        }
    }
    drop(request_tx);

    while !join_set.is_empty() {
        join_set.join_next().await;
    }
}

async fn fetch_traces(output: String, trace_rx: Receiver<Span>) {
    let mut traces = Vec::new();
    while let Ok(span) = trace_rx.recv() {
        traces.push(span);
    }
    let path = Path::new(&output);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).expect("Failed to create directory");
        }
    }
    let mut file = File::create(output).expect("Failed to create file");
    for trace in traces {
        let latency = trace.recv_at - trace.send_at;
        writeln!(file, "{}", latency).expect("Failed to write to file");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();
    eprintln!("args: {:?}", args);

    let (trace_tx, trace_rx) = unbounded();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                load_gen(args.rps, args.secs, args.concurrency, args.addr, trace_tx).await;
            });
    });

    let tracer = tokio::spawn(async move {
        fetch_traces(args.output, trace_rx).await;
    });
    tracer.await.expect("Failed to fetch traces");

    Ok(())
}
