use bridge::{
    tung_chung_client::TungChungClient,
    tung_chung_server::{TungChung, TungChungServer},
    WalkReply, WalkRequest,
};
use env_logger::{Builder, Env};
use futures_lite::future;
use hyper::rt::{Exec, Executor};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
use tokio::task::JoinHandle;
use tonic::{
    transport::{Channel, Server},
    Request, Response, Status,
};
use tonic_masa::DeadlineHint;
use tonic_masa::{Address, GlobalGraph, LocalGraph, Path};

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

#[derive(Debug)]
struct ExecImpl<'a> {
    ex: Arc<smol::Executor<'a>>,
}

impl<'a> ExecImpl<'a> {
    fn new(ex: Arc<smol::Executor<'a>>) -> Self {
        Self { ex }
    }

    async fn run(&self) {
        // [NOTE] Only a global queue is used in smol::Executor::tick().
        loop {
            self.ex.tick().await;
            // [TODO] Change to tokio yield.
            // [NOTE] Yield to tokio runtime.
            future::yield_now().await;
        }
    }
}

impl<'a, F> Executor<F> for ExecImpl<'a>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, fut: F, ddl: DeadlineHint) {
        // [NOTE] Deadline is passed from H2Stream.
        self.ex.spawn_with_ddl(fut, ddl).fallible().detach();
    }
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Server for benchmarking")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub n_threads: usize,
}

pub struct TungChungImpl {
    local_graphs: HashMap<Path, LocalGraph>,
    clients: HashMap<Path, TungChungClient<Channel>>,
}

impl TungChungImpl {
    pub fn new(
        local_graphs: HashMap<Path, LocalGraph>,
        clients: HashMap<Path, TungChungClient<Channel>>,
    ) -> Self {
        Self {
            local_graphs,
            clients,
        }
    }
}

fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
}

#[tonic::async_trait]
impl TungChung for TungChungImpl {
    // [TODO] Initialize servers in different processes.

    async fn say_frontend(
        &self,
        request: Request<WalkRequest>,
    ) -> Result<Response<WalkReply>, Status> {
        let start_at = time_now();
        let mut latency_spin = 0;

        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);
        ctx.set_local_graph(local_graph.clone());

        let spans = local_graph.spans();
        assert!(spans.len() == 4);

        let elapse = spans.first().unwrap().distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        for span in spans.iter().skip(1).take(spans.len() - 2) {
            let mut client = self.clients.get(span.path()).unwrap().clone();
            let mut request = Request::new(WalkRequest {
                name: "SayTungChung".to_string(),
            });
            request.metadata_mut().insert_ctx("par_ctx", &ctx);
            if span.path() == "/bridge.TungChung/SaySearch" {
                client.say_search(request).await.unwrap();
            } else if span.path() == "/bridge.TungChung/SayProfile" {
                client.say_profile(request).await.unwrap();
            } else {
                panic!("Invalid path: {:?}", span.path());
            }
        }

        let elapse = spans.last().unwrap().distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        let finish_at = time_now();
        let latency = finish_at - start_at;

        // [OPTION] Log by probability.
        // if ctx.request_id() % 10 == 0 {
        if true {
            log::warn!(
                "say_frontend,{},{},{}",
                ctx.request_id(),
                latency_spin,
                latency
            );
        }

        let reply = WalkReply {
            message: format!("Walk {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_search(
        &self,
        request: Request<WalkRequest>,
    ) -> Result<Response<WalkReply>, Status> {
        let start_at = time_now();
        let mut latency_spin = 0;

        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);
        ctx.set_local_graph(local_graph.clone());

        let spans = local_graph.spans();
        assert!(spans.len() == 2);

        let elapse = spans.first().unwrap().distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        let elapse = spans.last().unwrap().distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        let finish_at = time_now();
        let latency = finish_at - start_at;

        // [OPTION] Log by probability.
        // if ctx.request_id() % 10 == 0 {
        if true {
            log::warn!(
                "say_search,{},{},{}",
                ctx.request_id(),
                latency_spin,
                latency
            );
        }

        let reply = WalkReply {
            message: format!("Walk {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_profile(
        &self,
        request: Request<WalkRequest>,
    ) -> Result<Response<WalkReply>, Status> {
        let start_at = time_now();
        let mut latency_spin = 0;

        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);
        ctx.set_local_graph(local_graph.clone());

        let spans = local_graph.spans();
        assert!(spans.len() == 2);

        let elapse = spans.first().unwrap().distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        let elapse = spans.last().unwrap().distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        let finish_at = time_now();
        let latency = finish_at - start_at;

        // [OPTION] Log by probability.
        // if ctx.request_id() % 10 == 0 {
        if true {
            log::warn!(
                "say_profile,{},{},{}",
                ctx.request_id(),
                latency_spin,
                latency
            );
        }

        let reply = WalkReply {
            message: format!("Walk {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_geo(&self, _request: Request<WalkRequest>) -> Result<Response<WalkReply>, Status> {
        panic!("Not implemented");
    }

    async fn say_rate(
        &self,
        _request: Request<WalkRequest>,
    ) -> Result<Response<WalkReply>, Status> {
        panic!("Not implemented");
    }
}

#[derive(Debug, Clone)]
struct VirtualServer {
    addr: Address,
    conn_addrs: HashMap<Path, Address>,
    local_graphs: HashMap<Path, LocalGraph>,
    n_threads: usize,
}

impl VirtualServer {
    pub fn new(
        addr: Address,
        conn_addrs: HashMap<Path, Address>,
        local_graphs: HashMap<Path, LocalGraph>,
        n_threads: usize,
    ) -> Self {
        let mut paths = Vec::new();
        let mut addrs = Vec::new();
        for (path, addr) in conn_addrs.iter() {
            paths.push(path);
            addrs.push(addr);
        }
        assert_eq!(paths.len(), paths.iter().collect::<HashSet<_>>().len());
        assert_eq!(addrs.len(), addrs.iter().collect::<HashSet<_>>().len());
        Self {
            addr,
            conn_addrs,
            local_graphs,
            n_threads,
        }
    }

    pub fn addr(&self) -> &Address {
        &self.addr
    }

    pub fn local_graphs(&self) -> &HashMap<Path, LocalGraph> {
        &self.local_graphs
    }

    pub async fn get_clients(&self) -> HashMap<Path, TungChungClient<Channel>> {
        let mut clients = HashMap::new();
        for (path, addr) in self.conn_addrs.iter() {
            let client = TungChungClient::connect(addr.clone()).await.unwrap();
            clients.insert(path.clone(), client);
        }
        clients
    }

    pub fn n_threads(&self) -> usize {
        self.n_threads
    }
}

fn get_servers(args: Args, global_graph: GlobalGraph) -> Vec<VirtualServer> {
    let mut servers = Vec::new();

    let server_frontend = {
        let addr: Address = "[::1]:50051".to_string();
        let mut conn_addrs = HashMap::new();
        conn_addrs.insert(
            "/bridge.TungChung/SaySearch".to_string() as Path,
            "http://[::1]:50052".to_string() as Address,
        );
        conn_addrs.insert(
            "/bridge.TungChung/SayProfile".to_string() as Path,
            "http://[::1]:50053".to_string() as Address,
        );
        let path: Path = "/bridge.TungChung/SayFrontend".to_string();
        let local_graphs = {
            let mut graphs = HashMap::new();
            graphs.insert(
                global_graph.graph_id().clone(),
                global_graph.get_local_graph(&path).clone(),
            );
            graphs
        };
        let server = VirtualServer::new(addr, conn_addrs, local_graphs, args.n_threads);
        server
    };
    servers.push(server_frontend);

    let server_search = {
        let addr: Address = "[::1]:50052".to_string();
        let conn_addrs = HashMap::new();
        let path: Path = "/bridge.TungChung/SaySearch".to_string();
        let local_graphs = {
            let mut graphs = HashMap::new();
            graphs.insert(
                global_graph.graph_id().clone(),
                global_graph.get_local_graph(&path).clone(),
            );
            graphs
        };
        let server = VirtualServer::new(addr, conn_addrs, local_graphs, args.n_threads);
        server
    };
    servers.push(server_search);

    let server_profile = {
        let addr: Address = "[::1]:50053".to_string();
        let conn_addrs = HashMap::new();
        let path: Path = "/bridge.TungChung/SayProfile".to_string();
        let local_graphs = {
            let mut graphs = HashMap::new();
            graphs.insert(
                global_graph.graph_id().clone(),
                global_graph.get_local_graph(&path).clone(),
            );
            graphs
        };
        let server = VirtualServer::new(addr, conn_addrs, local_graphs, args.n_threads);
        server
    };
    servers.push(server_profile);

    servers.reverse();
    servers
}

async fn start_servers(servers: Vec<VirtualServer>) -> Vec<JoinHandle<()>> {
    let mut handles = Vec::new();

    for i in 0..servers.len() {
        let server = servers[i].clone();

        let ex = Arc::new(ExecImpl::new(Arc::new(smol::Executor::new())));
        for _ in 0..server.n_threads() {
            let ex = ex.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(ex.run());
            });
        }

        let h = tokio::spawn(async move {
            let addr = server.addr().parse().unwrap();
            let local_graphs = server.local_graphs().clone();
            let clients = server.get_clients().await;

            let tung_chung = TungChungImpl::new(local_graphs, clients);
            log::warn!("Listening on {}...", addr);
            Server::builder()
                .add_service(TungChungServer::new(tung_chung))
                .serve_with_executor(addr, Exec::Executor(ex))
                .await
                .unwrap();
        });
        handles.push(h);

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    handles
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    let global_graph = graph::get_global_graph_tung_chung();
    let servers = get_servers(args.clone(), global_graph.clone());
    let handles = start_servers(servers).await;
    for h in handles {
        h.await.unwrap();
    }

    Ok(())
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
    log::info!("Logging initialized");
}
