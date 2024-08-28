use bridge::{
    worker_client::WorkerClient,
    worker_server::{Worker, WorkerServer},
    HelloReply, HelloRequest,
};
use env_logger::{Builder, Env};
use futures_lite::future;
use hyper::rt::{Exec, Executor};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
use tonic::{
    transport::{Channel, Server},
    Request, Response, Status,
};
use tonic_masa::DeadlineHint;
use tonic_masa::{Address, LocalGraph, Path};

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
#[structopt(about = "Server for benchmarking")]
pub struct Args {
    #[structopt(short, long, default_value = "1")]
    pub n_threads: usize,
}

pub struct WorkerImpl {
    local_graphs: HashMap<Path, LocalGraph>,
    clients: HashMap<Path, WorkerClient<Channel>>,
}

impl WorkerImpl {
    pub fn new(
        local_graphs: HashMap<Path, LocalGraph>,
        clients: HashMap<Path, WorkerClient<Channel>>,
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
impl Worker for WorkerImpl {
    // [TODO] Initialize servers in different processes.

    async fn say_hello_i4(
        &self,
        _request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        panic!("Not implemented");
    }

    async fn say_hello_i3(
        &self,
        _request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        panic!("Not implemented");
    }

    async fn say_hello_i2(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);
        ctx.set_local_graph(local_graph.clone());

        let spans = local_graph.spans();
        let elapse = spans.first().unwrap().distribution().estimate();
        // .sample(ctx.request_id());
        let elapse_first = elapse;
        busy_spin(Duration::from_micros(elapse));

        let start_at = time_now();
        // for span in spans.iter().skip(1).take(spans.len() - 2) {
        //     let mut client = self.clients.get(span.path()).unwrap().clone();
        //     let mut request = Request::new(HelloRequest {
        //         name: "SayHelloI1".to_string(),
        //     });
        //     request.metadata_mut().insert_ctx("par_ctx", &ctx);
        //     client.say_hello_i1(request).await.unwrap();
        // }
        let finish_at = time_now();
        let latency = finish_at - start_at;

        // let elapse = spans.last().unwrap().distribution().estimate();
        // // .sample(ctx.request_id());
        // busy_spin(Duration::from_micros(elapse));

        // if ctx.request_id() % 10 == 0 {
        if true {
            log::warn!(
                "say_hello_i2,{},{},{}",
                ctx.request_id(),
                elapse_first,
                latency
            );
        }

        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_hello_i1(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let start_at = time_now();

        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);
        ctx.set_local_graph(local_graph.clone());

        let spans = local_graph.spans();
        let elapse = spans.first().unwrap().distribution().estimate();
        // .sample(ctx.request_id());
        let elapse_first = elapse;
        busy_spin(Duration::from_micros(elapse));

        // let elapse = spans.last().unwrap().distribution().estimate();
        // // .sample(ctx.request_id());
        // busy_spin(Duration::from_micros(elapse));

        let finish_at = time_now();
        let latency = finish_at - start_at;
        // if ctx.request_id() % 10 == 0 {
        if true {
            use log::warn;
            warn!(
                "say_hello_i1,{},{},{}",
                ctx.request_id(),
                elapse_first,
                latency
            );
        }

        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
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

    pub async fn get_clients(&self) -> HashMap<Path, WorkerClient<Channel>> {
        let mut clients = HashMap::new();
        for (path, addr) in self.conn_addrs.iter() {
            let client = WorkerClient::connect(addr.clone()).await.unwrap();
            clients.insert(path.clone(), client);
        }
        clients
    }

    pub fn n_threads(&self) -> usize {
        self.n_threads
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
    log::info!("Logging initialized");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();

    let global_graph = graph::get_global_graph();

    let mut servers = Vec::new();

    let server2 = {
        let addr: Address = "[::1]:50052".to_string();
        let conn_addrs = HashMap::new();
        let path: Path = "/bridge.Worker/SayHelloI1".to_string();
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
    servers.push(server2);

    let server1 = {
        let addr: Address = "[::1]:50051".to_string();
        let mut conn_addrs = HashMap::new();
        conn_addrs.insert(
            "/bridge.Worker/SayHelloI1".to_string() as Path,
            "http://[::1]:50052".to_string() as Address,
        );
        let path: Path = "/bridge.Worker/SayHelloI2".to_string();
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
    servers.push(server1);

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

            let worker = WorkerImpl::new(local_graphs, clients);
            log::info!("Listening on {}...", addr);
            Server::builder()
                .add_service(WorkerServer::new(worker))
                .serve_with_executor(addr, Exec::Executor(ex))
                .await
                .unwrap();
        });
        handles.push(h);

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}
