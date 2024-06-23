use crossbeam_channel::{unbounded, Receiver, Sender};
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp, Gamma, Uniform};
use std::collections::BinaryHeap;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::{
    atomic::{AtomicI32, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
use tokio::time::{Duration, Instant};

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Rivas for simulation")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub depth: usize,
    #[structopt(short, long, required = true)]
    pub exec_ks: Vec<f64>,
    #[structopt(short, long, required = true)]
    pub exec_mus: Vec<u64>,
    #[structopt(short, long, required = true)]
    pub replicas: u64,
    #[structopt(short, long, required = true)]
    pub mode: String,
    #[structopt(short, long, required = true)]
    pub rps: u64,
    #[structopt(short, long, required = true)]
    pub secs: u64,
    #[structopt(short, long, required = true)]
    pub concurrency: u64,
    #[structopt(short, long, required = true)]
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct Request {
    send_at: u64,
    finish_at: u64,
    prev_elapse: u64,
    proc_elapses: Vec<u64>,
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

fn consume(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
}

#[derive(Debug)]
struct TxManager {
    rng: StdRng,
    txs: Vec<Sender<Request>>,
    uniform: Uniform<usize>,
}

impl TxManager {
    fn new(seed: u64) -> Self {
        TxManager {
            rng: StdRng::seed_from_u64(seed),
            txs: Vec::new(),
            uniform: Uniform::new(0, 1),
        }
    }

    fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    fn add(&mut self, tx: Sender<Request>) {
        self.txs.push(tx);
        self.uniform = Uniform::new(0, self.txs.len());
    }

    // fn send(&mut self, request: Request) {
    //     let index = self.uniform.sample(&mut self.rng);
    //     self.txs[index].send(request).unwrap();
    // }

    fn try_send(&mut self, request: Request) -> bool {
        let index = self.uniform.sample(&mut self.rng);
        self.txs[index].try_send(request).is_ok()
    }
}

#[derive(Debug)]
struct Client {
    rng: StdRng,
    depth: usize,
    exec_ks: Vec<f64>,
    exec_mus: Vec<u64>,
    rps: u64,
    secs: u64,
    token: Arc<AtomicI32>,
    tx_manager: TxManager,
}

impl Client {
    fn new(
        seed: u64,
        depth: usize,
        exec_ks: Vec<f64>,
        exec_mus: Vec<u64>,
        rps: u64,
        secs: u64,
        token: Arc<AtomicI32>,
    ) -> Self {
        Client {
            rng: StdRng::seed_from_u64(seed),
            depth,
            exec_ks,
            exec_mus,
            rps,
            secs,
            token,
            tx_manager: TxManager::new(seed),
        }
    }

    async fn start_client(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let start_at = Instant::now();
        let pause_at = start_at + Duration::from_secs(self.secs);

        let mut elapse = 0f64;

        let exponential = Exp::new(self.rps as f64).unwrap();
        let mut distributions = Vec::new();
        for i in 0..self.depth {
            // let normal = Normal::from_mean_cv(self.exec_mus[i] as f64, 0.3).unwrap();
            let k = self.exec_ks[i] as f64;
            let mu = self.exec_mus[i] as f64;
            let theta = mu / k;
            let gamma = Gamma::new(k, theta).unwrap();
            distributions.push(gamma);
        }

        loop {
            if Instant::now() > pause_at {
                break;
            }

            if self.token.load(Ordering::SeqCst) <= 0 {
                continue;
            }

            let send_at = start_at + Duration::from_secs_f64(elapse);
            tokio::time::sleep_until(send_at).await;

            let value = { exponential.sample(&mut self.rng) };
            elapse += value;

            let send_at = time_now();
            let prev_elapse = 0;
            let hint = send_at - prev_elapse;
            let mut proc_elapses = Vec::new();
            for i in 0..self.depth {
                let mut elapse = distributions[i].sample(&mut self.rng) as u64;
                elapse = elapse.max(0);
                proc_elapses.push(elapse);
            }
            let request = Request {
                send_at,
                finish_at: 0,
                prev_elapse,
                proc_elapses,
                total_elapse: 0,
                hint,
            };

            if self.token.load(Ordering::SeqCst) > 0 {
                if self.tx_manager.try_send(request) {
                    self.token.fetch_sub(1, Ordering::SeqCst);
                }
            }
        }

        println!("Client completed");
        Ok(())
    }
}

#[derive(Debug)]
struct Server {
    depth: usize,
    mode: String,
    secs: u64,
    token: Arc<AtomicI32>,
    tx: Sender<Request>,
    rx: Receiver<Request>,
    tx_manager: TxManager,
    trace_tx: Sender<u64>,
}

impl Server {
    fn new(
        seed: u64,
        depth: usize,
        mode: String,
        secs: u64,
        token: Arc<AtomicI32>,
        trace_tx: Sender<u64>,
    ) -> Self {
        let (tx, rx) = unbounded::<Request>();
        Server {
            depth,
            mode,
            secs,
            token,
            tx,
            rx,
            tx_manager: TxManager::new(seed),
            trace_tx,
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
                if let Ok(mut request) = self.rx.try_recv() {
                    let elapse = request.proc_elapses[self.depth];
                    consume(Duration::from_micros(elapse));
                    let is_leaf = self.tx_manager.is_empty();
                    if !is_leaf {
                        self.tx_manager.try_send(request);
                    } else {
                        request.finish_at = time_now();
                        request.total_elapse =
                            request.finish_at - request.send_at + request.prev_elapse;
                        self.trace_tx.try_send(request.total_elapse).unwrap();
                        self.token.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
        }

        println!("Server {} completed", self.depth);
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
                if let Ok(request) = self.rx.try_recv() {
                    heap.push(request);
                }
                continue;
            }
            if let Some(mut request) = heap.pop() {
                let elapse = request.proc_elapses[self.depth];
                consume(Duration::from_micros(elapse));
                let is_leaf = self.tx_manager.is_empty();
                if !is_leaf {
                    self.tx_manager.try_send(request);
                } else {
                    request.finish_at = time_now();
                    request.total_elapse =
                        request.finish_at - request.send_at + request.prev_elapse;
                    self.trace_tx.try_send(request.total_elapse).unwrap();
                    self.token.fetch_add(1, Ordering::SeqCst);
                }
            }
        }

        println!("Server {} completed", self.depth);
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
    const KEY: u64 = 13;
    const SEED: u64 = 998244353;

    let mut args = Args::from_args();
    args.concurrency = args.concurrency * args.replicas;

    let token = Arc::new(AtomicI32::new(args.concurrency as i32));
    let (trace_tx, trace_rx) = unbounded();

    let seed = SEED * KEY + args.rps;
    let mut rng = StdRng::seed_from_u64(seed);
    let uniform = Uniform::new(0, 1 << 63);

    let mut client = Client::new(
        uniform.sample(&mut rng),
        args.depth,
        args.exec_ks,
        args.exec_mus,
        args.rps,
        args.secs,
        token.clone(),
    );

    let mut depth_0_servers = Vec::new();
    for _ in 0..args.replicas {
        let server = Server::new(
            uniform.sample(&mut rng),
            0,
            args.mode.clone(),
            args.secs,
            token.clone(),
            trace_tx.clone(),
        );
        client.tx_manager.add(server.tx.clone());
        depth_0_servers.push(server);
    }

    let mut depth_1_servers = Vec::new();
    for _ in 0..args.replicas {
        let server = Server::new(
            uniform.sample(&mut rng),
            1,
            args.mode.clone(),
            args.secs,
            token.clone(),
            trace_tx.clone(),
        );
        for depth_0_server in depth_0_servers.iter_mut() {
            depth_0_server.tx_manager.add(server.tx.clone());
        }
        depth_1_servers.push(server);
    }

    let mut depth_2_servers = Vec::new();
    for _ in 0..args.replicas {
        let server = Server::new(
            uniform.sample(&mut rng),
            2,
            args.mode.clone(),
            args.secs,
            token.clone(),
            trace_tx.clone(),
        );
        for depth_1_server in depth_1_servers.iter_mut() {
            depth_1_server.tx_manager.add(server.tx.clone());
        }
        depth_2_servers.push(server);
    }

    let mut depth_3_servers = Vec::new();
    for _ in 0..args.replicas {
        let server = Server::new(
            uniform.sample(&mut rng),
            3,
            args.mode.clone(),
            args.secs,
            token.clone(),
            trace_tx.clone(),
        );
        for depth_2_server in depth_2_servers.iter_mut() {
            depth_2_server.tx_manager.add(server.tx.clone());
        }
        depth_3_servers.push(server);
    }

    let mut depth_4_servers = Vec::new();
    for _ in 0..args.replicas {
        let server = Server::new(
            uniform.sample(&mut rng),
            4,
            args.mode.clone(),
            args.secs,
            token.clone(),
            trace_tx.clone(),
        );
        for depth_3_server in depth_3_servers.iter_mut() {
            depth_3_server.tx_manager.add(server.tx.clone());
        }
        depth_4_servers.push(server);
    }

    let mut depth_5_servers = Vec::new();
    for _ in 0..args.replicas {
        let server = Server::new(
            uniform.sample(&mut rng),
            5,
            args.mode.clone(),
            args.secs,
            token.clone(),
            trace_tx.clone(),
        );
        for depth_4_server in depth_4_servers.iter_mut() {
            depth_4_server.tx_manager.add(server.tx.clone());
        }
        depth_5_servers.push(server);
    }

    let mut handles = Vec::new();

    handles.push(tokio::spawn(async move {
        client.start_client().await.unwrap();
    }));

    handles.extend(depth_0_servers.into_iter().map(|mut server| {
        tokio::spawn(async move {
            server.start_server().await.unwrap();
        })
    }));

    handles.extend(depth_1_servers.into_iter().map(|mut server| {
        tokio::spawn(async move {
            server.start_server().await.unwrap();
        })
    }));

    handles.extend(depth_2_servers.into_iter().map(|mut server| {
        tokio::spawn(async move {
            server.start_server().await.unwrap();
        })
    }));

    handles.extend(depth_3_servers.into_iter().map(|mut server| {
        tokio::spawn(async move {
            server.start_server().await.unwrap();
        })
    }));

    handles.extend(depth_4_servers.into_iter().map(|mut server| {
        tokio::spawn(async move {
            server.start_server().await.unwrap();
        })
    }));

    handles.extend(depth_5_servers.into_iter().map(|mut server| {
        tokio::spawn(async move {
            server.start_server().await.unwrap();
        })
    }));

    handles.push(tokio::spawn(async move {
        fetch_traces(args.output, trace_rx).await;
    }));

    drop(trace_tx);
    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}
