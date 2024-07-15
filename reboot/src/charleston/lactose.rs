use crossbeam_channel::{unbounded, Receiver, Sender};
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp, Gamma, Uniform};
use std::collections::{BinaryHeap, VecDeque};
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
    pub first_exec_mus: Vec<u64>,
    #[structopt(short, long, required = true)]
    pub second_exec_mus: Vec<u64>,
    #[structopt(short, long, required = true)]
    pub first_slo: u64,
    #[structopt(short, long, required = true)]
    pub second_slo: u64,
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
    recv_at: u64,
    finish_at: u64,
    proc_mus: Vec<u64>,
    proc_elapses: Vec<u64>,
    slo: u64,
    hint: u64,
}

#[derive(Debug, Clone)]
pub struct Span {
    span: String,
    slo: u64,
    latency: u64,
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
    first_exec_mus: Vec<u64>,
    second_exec_mus: Vec<u64>,
    first_slo: u64,
    second_slo: u64,
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
        first_exec_mus: Vec<u64>,
        second_exec_mus: Vec<u64>,
        first_slo: u64,
        second_slo: u64,
        rps: u64,
        secs: u64,
        token: Arc<AtomicI32>,
    ) -> Self {
        Client {
            rng: StdRng::seed_from_u64(seed),
            depth,
            exec_ks,
            first_exec_mus,
            second_exec_mus,
            first_slo,
            second_slo,
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
        let uniform = Uniform::new(0, 1_000_000_007);

        let exponential = Exp::new(self.rps as f64).unwrap();
        let mut first_gammas = Vec::new();
        let mut second_gammas = Vec::new();
        for i in 0..self.depth {
            let k = self.exec_ks[i] as f64;

            let mu = self.first_exec_mus[i] as f64;
            let theta = mu / k;
            let gamma = Gamma::new(k, theta).unwrap();
            first_gammas.push(gamma);

            let mu = self.second_exec_mus[i] as f64;
            let theta = mu / k;
            let gamma = Gamma::new(k, theta).unwrap();
            second_gammas.push(gamma);
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

            let coin = uniform.sample(&mut self.rng) % 2;
            let send_at = time_now();
            // [NOTE] hint = send_at + SLO.
            let slo = {
                if coin == 0 {
                    self.first_slo
                } else {
                    self.second_slo
                }
            };
            let mut hint = send_at + slo;
            let mut proc_mus = Vec::new();
            let mut proc_elapses = Vec::new();
            for i in 0..self.depth {
                let proc_mu = {
                    if coin == 0 {
                        self.first_exec_mus[i]
                    } else {
                        self.second_exec_mus[i]
                    }
                };
                proc_mus.push(proc_mu);
                hint -= proc_mu;

                let elapse = {
                    if coin == 0 {
                        first_gammas[i].sample(&mut self.rng) as u64
                    } else {
                        second_gammas[i].sample(&mut self.rng) as u64
                    }
                };
                proc_elapses.push(elapse);
            }

            let request = Request {
                send_at,
                recv_at: 0,
                finish_at: 0,
                proc_mus,
                proc_elapses,
                slo,
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
    trace_tx: Sender<Span>,
}

impl Server {
    fn new(
        seed: u64,
        depth: usize,
        mode: String,
        secs: u64,
        token: Arc<AtomicI32>,
        trace_tx: Sender<Span>,
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

        let mut queue = VecDeque::new();
        loop {
            if Instant::now() > pause_at {
                break;
            }
            while !self.rx.is_empty() {
                if let Ok(mut request) = self.rx.try_recv() {
                    request.recv_at = time_now();
                    queue.push_back(request);
                }
                continue;
            }
            if let Some(mut request) = queue.pop_front() {
                let elapse = request.proc_elapses[self.depth];
                consume(Duration::from_micros(elapse));
                let is_leaf = self.tx_manager.is_empty();
                if !is_leaf {
                    self.tx_manager.try_send(request);
                } else {
                    request.finish_at = time_now();
                    let span = Span {
                        span: "end_to_end".to_string(),
                        slo: request.slo,
                        latency: request.finish_at - request.send_at,
                    };
                    self.trace_tx.try_send(span).unwrap();
                    let span = Span {
                        span: "service".to_string(),
                        slo: request.slo,
                        latency: request.finish_at - request.recv_at,
                    };
                    self.trace_tx.try_send(span).unwrap();
                    self.token.fetch_add(1, Ordering::SeqCst);
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
                if let Ok(mut request) = self.rx.try_recv() {
                    request.recv_at = time_now();
                    request.hint += request.proc_mus[self.depth];
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
                    let span = Span {
                        span: "end_to_end".to_string(),
                        slo: request.slo,
                        latency: request.finish_at - request.send_at,
                    };
                    self.trace_tx.try_send(span).unwrap();
                    let span = Span {
                        span: "service".to_string(),
                        slo: request.slo,
                        latency: request.finish_at - request.recv_at,
                    };
                    self.trace_tx.try_send(span).unwrap();
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

async fn fetch_traces(output: String, trace_rx: Receiver<Span>) {
    let path = Path::new(&output);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).unwrap();
        }
    }
    let mut file = File::create(output).unwrap();
    while let Ok(span) = trace_rx.recv() {
        writeln!(file, "{},{},{}", span.span, span.slo, span.latency).unwrap();
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
        args.first_exec_mus,
        args.second_exec_mus,
        args.first_slo,
        args.second_slo,
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

    let mut depth_6_servers = Vec::new();
    for _ in 0..args.replicas {
        let server = Server::new(
            uniform.sample(&mut rng),
            6,
            args.mode.clone(),
            args.secs,
            token.clone(),
            trace_tx.clone(),
        );
        for depth_5_server in depth_5_servers.iter_mut() {
            depth_5_server.tx_manager.add(server.tx.clone());
        }
        depth_6_servers.push(server);
    }

    let mut depth_7_servers = Vec::new();
    for _ in 0..args.replicas {
        let server = Server::new(
            uniform.sample(&mut rng),
            7,
            args.mode.clone(),
            args.secs,
            token.clone(),
            trace_tx.clone(),
        );
        for depth_6_server in depth_6_servers.iter_mut() {
            depth_6_server.tx_manager.add(server.tx.clone());
        }
        depth_7_servers.push(server);
    }

    let mut depth_8_servers = Vec::new();
    for _ in 0..args.replicas {
        let server = Server::new(
            uniform.sample(&mut rng),
            8,
            args.mode.clone(),
            args.secs,
            token.clone(),
            trace_tx.clone(),
        );
        for depth_7_server in depth_7_servers.iter_mut() {
            depth_7_server.tx_manager.add(server.tx.clone());
        }
        depth_8_servers.push(server);
    }

    let mut depth_9_servers = Vec::new();
    for _ in 0..args.replicas {
        let server = Server::new(
            uniform.sample(&mut rng),
            9,
            args.mode.clone(),
            args.secs,
            token.clone(),
            trace_tx.clone(),
        );
        for depth_8_server in depth_8_servers.iter_mut() {
            depth_8_server.tx_manager.add(server.tx.clone());
        }
        depth_9_servers.push(server);
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

    handles.extend(depth_6_servers.into_iter().map(|mut server| {
        tokio::spawn(async move {
            server.start_server().await.unwrap();
        })
    }));

    handles.extend(depth_7_servers.into_iter().map(|mut server| {
        tokio::spawn(async move {
            server.start_server().await.unwrap();
        })
    }));

    handles.extend(depth_8_servers.into_iter().map(|mut server| {
        tokio::spawn(async move {
            server.start_server().await.unwrap();
        })
    }));

    handles.extend(depth_9_servers.into_iter().map(|mut server| {
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
