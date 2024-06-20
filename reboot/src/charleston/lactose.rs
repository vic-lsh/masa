use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp, Normal};
use std::collections::BinaryHeap;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::{
    atomic::{AtomicI8, Ordering},
    Arc,
};
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

#[derive(Debug)]
struct TxManager {
    txs: Vec<Sender<Request>>,
    index: usize,
}

impl TxManager {
    fn new() -> Self {
        TxManager {
            txs: Vec::new(),
            index: 0,
        }
    }

    fn add(&mut self, tx: Sender<Request>) {
        self.txs.push(tx);
    }

    fn send(&mut self, request: Request) {
        self.txs[self.index].send(request).unwrap();
        self.index = (self.index + 1) % self.txs.len();
    }
}

#[derive(Debug)]
struct Client {
    rps: u64,
    secs: u64,
    token: Arc<AtomicI8>,
    tx_manager: TxManager,
}

impl Client {
    fn new(rps: u64, secs: u64, token: Arc<AtomicI8>) -> Self {
        Client {
            rps,
            secs,
            token,
            tx_manager: TxManager::new(),
        }
    }

    async fn start_client(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let start_at = Instant::now();
        let pause_at = start_at + Duration::from_secs(self.secs);

        let mut elapse = 0f64;

        let mut rng = StdRng::seed_from_u64(998244353);
        let exponential = Exp::new(self.rps as f64).unwrap();
        let normal = Normal::new(ELAPSE_MU as f64, ELAPSE_SIGMA as f64).unwrap();

        loop {
            let now = Instant::now();
            if now > pause_at {
                break;
            }
            let send_at = start_at + Duration::from_secs_f64(elapse);
            tokio::time::sleep_until(send_at).await;

            let value = {
                // let mut rng = rand::thread_rng();
                exponential.sample(&mut rng)
            };
            elapse += value;

            let send_at = time_now();
            let prev_elapse = {
                // let mut rng = rand::thread_rng();
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

            while self.token.load(Ordering::SeqCst) <= 0 {
                tokio::task::yield_now().await;
            }
            self.token.fetch_sub(1, Ordering::SeqCst);
            self.tx_manager.send(request);
        }

        println!("Client completed");
        Ok(())
    }
}

#[derive(Debug)]
struct BrotherServer {
    mode: String,
    secs: u64,
    tx: Sender<Request>,
    rx: Receiver<Request>,
    tx_manager: TxManager,
}

impl BrotherServer {
    fn new(mode: String, secs: u64) -> Self {
        let (tx, rx) = unbounded::<Request>();
        BrotherServer {
            mode,
            secs,
            tx,
            rx,
            tx_manager: TxManager::new(),
        }
    }

    async fn start_fcfs_server(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let start_at = Instant::now();
        let pause_at = start_at + Duration::from_secs(self.secs + 3);

        loop {
            if Instant::now() > pause_at {
                break;
            }
            if !self.rx.is_empty() {
                if let Ok(mut request) = self.rx.recv() {
                    busy_spin(Duration::from_micros(EXEC_MU));
                    request.finish_at = time_now();
                    request.total_elapse =
                        request.finish_at - request.send_at + request.prev_elapse;
                    self.tx_manager.send(request);
                    tokio::task::yield_now().await;
                }
            }
        }

        println!("Brother server completed");
        Ok(())
    }

    async fn start_masa_server(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let start_at = Instant::now();
        let pause_at = start_at + Duration::from_secs(self.secs + 3);

        let mut heap = BinaryHeap::new();
        loop {
            if Instant::now() > pause_at {
                break;
            }
            while !self.rx.is_empty() {
                match self.rx.try_recv() {
                    Ok(request) => heap.push(request),
                    Err(_) => break,
                }
            }
            if let Some(mut req) = heap.pop() {
                busy_spin(Duration::from_micros(EXEC_MU));
                req.finish_at = time_now();
                req.total_elapse = req.finish_at - req.send_at + req.prev_elapse;
                self.tx_manager.send(req);
                tokio::task::yield_now().await;
            }
        }

        println!("Server completed");
        Ok(())
    }

    async fn start_server(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        match self.mode.as_str() {
            "fcfs" => self.start_fcfs_server().await,
            "masa" => self.start_masa_server().await,
            _ => panic!("Invalid mode"),
        }
    }
}

#[derive(Debug)]
struct LeafServer {
    mode: String,
    secs: u64,
    token: Arc<AtomicI8>,
    tx: Sender<Request>,
    rx: Receiver<Request>,
    trace_tx: Sender<u64>,
}

impl LeafServer {
    fn new(mode: String, secs: u64, token: Arc<AtomicI8>, trace_tx: Sender<u64>) -> Self {
        let (tx, rx) = unbounded::<Request>();
        LeafServer {
            mode,
            secs,
            token,
            tx,
            rx,
            trace_tx,
        }
    }

    async fn start_fcfs_server(&self) -> Result<(), Box<dyn std::error::Error>> {
        let start_at = Instant::now();
        let pause_at = start_at + Duration::from_secs(self.secs + 3);

        loop {
            if Instant::now() > pause_at {
                break;
            }
            if !self.rx.is_empty() {
                if let Ok(mut request) = self.rx.recv() {
                    busy_spin(Duration::from_micros(EXEC_MU));
                    request.finish_at = time_now();
                    request.total_elapse =
                        request.finish_at - request.send_at + request.prev_elapse;
                    self.trace_tx.send(request.total_elapse).unwrap();
                    self.token.fetch_add(1, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                }
            }
        }

        println!("Leaf server completed");
        Ok(())
    }

    async fn start_masa_server(&self) -> Result<(), Box<dyn std::error::Error>> {
        let start_at = Instant::now();
        let pause_at = start_at + Duration::from_secs(self.secs + 3);

        let mut heap = BinaryHeap::new();
        loop {
            if Instant::now() > pause_at {
                break;
            }
            while !self.rx.is_empty() {
                match self.rx.try_recv() {
                    Ok(request) => heap.push(request),
                    Err(_) => break,
                }
            }
            if let Some(mut req) = heap.pop() {
                busy_spin(Duration::from_micros(EXEC_MU));
                req.finish_at = time_now();
                req.total_elapse = req.finish_at - req.send_at + req.prev_elapse;
                self.trace_tx.send(req.total_elapse).unwrap();
                self.token.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
        }

        println!("Server completed");
        Ok(())
    }

    async fn start_server(&self) -> Result<(), Box<dyn std::error::Error>> {
        match self.mode.as_str() {
            "fcfs" => self.start_fcfs_server().await,
            "masa" => self.start_masa_server().await,
            _ => panic!("Invalid mode"),
        }
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

    let token = Arc::new(AtomicI8::new(args.concurrency as i8));
    let (trace_tx, trace_rx) = unbounded();

    let mut client = Client::new(args.rps, args.secs, token.clone());
    let mut bro_server = BrotherServer::new(args.mode.clone(), args.secs);
    let leaf_server = LeafServer::new(args.mode.clone(), args.secs, token, trace_tx);
    client.tx_manager.add(bro_server.tx.clone());
    bro_server.tx_manager.add(leaf_server.tx.clone());

    let client_handle = tokio::spawn(async move {
        client.start_client().await.unwrap();
    });
    let bro_server_handle = tokio::spawn(async move {
        bro_server.start_server().await.unwrap();
    });
    let leaf_server_handle = tokio::spawn(async move {
        leaf_server.start_server().await.unwrap();
    });
    let tracer_handle = tokio::spawn(async move {
        fetch_traces(args.output, trace_rx).await;
    });

    let handles = vec![
        client_handle,
        bro_server_handle,
        leaf_server_handle,
        tracer_handle,
    ];
    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}
