use crossbeam_channel::{bounded, unbounded, Receiver, Sender, TryRecvError};
use rand_distr::{Distribution, Exp, Normal};
use std::collections::BinaryHeap;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
use tokio::time::{Duration, Instant};

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Rivas for simulation")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub mode: String,
    #[structopt(short, long, required = true)]
    pub rps: u64,
    #[structopt(short, long, required = true)]
    pub secs: u64,
    #[structopt(short, long, required = true)]
    pub concurrency: u32,
    #[structopt(short, long, required = true)]
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct Request {
    send_at: u64,
    finish_at: u64,
    prev_elapse: u64,
    total_elapse: u64,
    hint: u64,
}

impl Ord for Request {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.hint.cmp(&self.hint)
    }
}

impl PartialOrd for Request {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(other.hint.cmp(&self.hint))
    }
}

impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        self.hint == other.hint
    }
}

impl Eq for Request {}

pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
}

const ELAPSE_MU: u64 = 20_000; // 20ms
const ELAPSE_SIGMA: u64 = 5_000; // 5ms
const EXEC_MU: u64 = 2_000; // 2ms

async fn start_client(
    rps: u64,
    secs: u64,
    concurrency: u32,
    channel_tx: Sender<Request>,
) -> Result<(), Box<dyn std::error::Error>> {
    let start_at = Instant::now();
    let pause_at = start_at + Duration::from_secs(secs);

    let mut elapse = 0f64;
    let exponential = Exp::new(rps as f64).unwrap();

    let (request_tx, request_rx) = bounded(concurrency as usize);

    let mut handles = Vec::new();
    let handle = tokio::spawn(async move {
        let normal = Normal::new(ELAPSE_MU as f64, ELAPSE_SIGMA as f64).unwrap();

        loop {
            let now = Instant::now();
            if now > pause_at {
                break;
            }
            let send_at = start_at + Duration::from_secs_f64(elapse);
            tokio::time::sleep_until(send_at).await;
            let value = {
                let mut rng = rand::thread_rng();
                exponential.sample(&mut rng)
            };
            elapse += value;
            let send_at = time_now();
            let prev_elapse = {
                let mut rng = rand::thread_rng();
                normal.sample(&mut rng) as u64
            };
            let hint = send_at - prev_elapse;
            let request = Request {
                send_at,
                finish_at: 0,
                prev_elapse,
                total_elapse: 0,
                hint,
            };
            while request_tx.is_full() {
                tokio::task::yield_now().await;
            }
            request_tx.send(request).unwrap();
        }
    });
    handles.push(handle);

    for _ in 0..concurrency {
        let channel_tx = channel_tx.clone();
        let request_rx = request_rx.clone();
        let handle = tokio::spawn(async move {
            loop {
                match request_rx.try_recv() {
                    Ok(request) => {
                        channel_tx.send(request).unwrap();
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
        handles.push(handle);
    }

    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}

async fn start_fcfs_server(
    secs: u64,
    channel_rx: Receiver<Request>,
    trace_tx: Sender<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let start_at = Instant::now();
    let pause_at = start_at + Duration::from_secs(secs + 3);

    loop {
        if Instant::now() > pause_at {
            break;
        }
        if let Ok(mut req) = channel_rx.try_recv() {
            busy_spin(Duration::from_micros(EXEC_MU));
            req.finish_at = time_now();
            req.total_elapse = req.finish_at - req.send_at + req.prev_elapse;
            trace_tx.send(req.total_elapse).unwrap();
        }
    }

    Ok(())
}

async fn start_ddl_server(
    secs: u64,
    channel_rx: Receiver<Request>,
    trace_tx: Sender<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let start_at = Instant::now();
    let pause_at = start_at + Duration::from_secs(secs + 3);

    let mut heap = BinaryHeap::new();

    loop {
        if Instant::now() > pause_at {
            break;
        }
        while !channel_rx.is_empty() {
            match channel_rx.try_recv() {
                Ok(req) => {
                    heap.push(req);
                }
                Err(_) => {
                    break;
                }
            }
        }
        if let Some(mut req) = heap.pop() {
            busy_spin(Duration::from_micros(EXEC_MU));
            req.finish_at = time_now();
            req.total_elapse = req.finish_at - req.send_at + req.prev_elapse;
            trace_tx.send(req.total_elapse).unwrap();
        }
    }

    Ok(())
}

async fn start_server(
    mode: &str,
    secs: u64,
    channel_rx: Receiver<Request>,
    trace_tx: Sender<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    match mode {
        "fcfs" => start_fcfs_server(secs, channel_rx, trace_tx).await,
        "ddl" => start_ddl_server(secs, channel_rx, trace_tx).await,
        _ => panic!("Invalid mode"),
    }
}

async fn fetch_traces(output: String, trace_rx: Receiver<u64>) {
    let path = Path::new(&output);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).unwrap();
        }
    }
    let mut file = File::create(output).unwrap();
    while let Ok(latency) = trace_rx.recv() {
        writeln!(file, "{}", latency).unwrap();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();

    let (channel_tx, channel_rx) = unbounded();
    let (trace_tx, trace_rx) = unbounded();
    let client = tokio::spawn(async move {
        start_client(args.rps, args.secs, args.concurrency, channel_tx)
            .await
            .unwrap();
    });
    let server = tokio::spawn(async move {
        start_server(&args.mode, args.secs, channel_rx, trace_tx)
            .await
            .unwrap();
    });
    let trace = tokio::spawn(async move {
        fetch_traces(args.output, trace_rx).await;
    });

    let handles = vec![client, server, trace];
    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}
