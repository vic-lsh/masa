pub mod hello {
    tonic::include_proto!("hello");
}
mod graph;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use env_logger::{Builder, Env};

// [TODO] Move to `exec.rs`.
use futures_lite::future;
use tonic_masa::PriorityHint;

use hyper::rt::{Exec, Executor};
use structopt::StructOpt;
use tonic::{
    masa::AsyncTaskMetadata,
    transport::{Channel, Server},
    Request, Response, Status,
};
use tonic_masa::{
    Address, Context, GlobalGraph, Latency, LocalGraph, LocalGraphTracker, Path, FIFO, FIFO_TWO,
    ONLINE_TRACKER, PRIO_GLOBAL, PRIO_LOCAL,
};

use hello::{
    greeter_client::GreeterClient,
    greeter_server::{Greeter, GreeterServer},
    HelloReply, HelloRequest,
};

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

pub struct GreeterImpl<'a> {
    local_graphs: HashMap<Path, LocalGraph>,
    local_graph_trackers: HashMap<Path, RwLock<LocalGraphTracker>>,
    clients: HashMap<Path, GreeterClient<Channel>>,
    executor: Arc<ExecImpl<'a>>,
}

impl<'a> GreeterImpl<'a> {
    pub fn new(
        local_graphs: HashMap<Path, LocalGraph>,
        clients: HashMap<Path, GreeterClient<Channel>>,
        executor: Arc<ExecImpl<'a>>,
    ) -> Self {
        let local_graph_trackers = local_graphs
            .iter()
            .map(|(path, local_graph)| {
                let local_graph = LocalGraphTracker::from(local_graph.clone());
                (path.clone(), RwLock::new(local_graph))
            })
            .collect();
        Self {
            local_graphs,
            local_graph_trackers,
            clients,
            executor,
        }
    }

    fn set_child_ctx(&self, ctx: &Context, request: &mut Request<HelloRequest>, path: &Path) {
        let graph = self
            .local_graph_trackers
            .get(ctx.graph_id())
            .unwrap()
            .read()
            .unwrap();
        let deadline;
        let latest_exec_at;
        if PRIO_LOCAL {
            deadline = ctx.deadline() - graph.estimate_suffix_deadline(path);
            latest_exec_at = ctx.deadline() - graph.estimate_suffix_latest_exec_at(path);
        } else if PRIO_GLOBAL || FIFO_TWO || FIFO {
            deadline = ctx.deadline();
            latest_exec_at = ctx.latest_exec_at();
        } else {
            panic!("Unimplemented policy");
        }
        let child_ctx = Context::new(
            ctx.graph_id().clone(),
            ctx.request_id(),
            deadline,
            latest_exec_at,
            ctx.request_class(),
        );
        request.metadata_mut().insert_ctx("ctx", &child_ctx);
    }

    fn track_span(&self, ctx: &Context, path: &Path, latency: Latency) {
        // if ONLINE_TRACKER {
        //     let mut graph = self
        //         .local_graph_trackers
        //         .get(ctx.graph_id())
        //         .unwrap()
        //         .write()
        //         .unwrap();
        //     graph.track_span(path, latency);
        // }
    }
}

fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
}

#[tonic::async_trait]
impl Greeter for GreeterImpl<'static> {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        self.say_hello_fanout(request).await
    }

    async fn say_hola(
        &self,
        _request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        panic!("Not implemented");
    }

    async fn say_goodbye(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let ctx = request.metadata().get_ctx("ctx").unwrap();
        let graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("say_goodbye, ddl: {:?}", async_task::get_task_ddl());

        let spans = graph.spans();
        assert!(spans.len() == 2);

        for i in 0..spans.len() {
            let span = &spans[i];
            let path = span.path();

            if i == 0 || i == spans.len() - 1 {
                let elapse = span.distribution().sample(ctx.request_id());
                let start_at = time_now();
                busy_spin(Duration::from_micros(elapse));
                let latency = time_now() - start_at;
                self.track_span(&ctx, path, latency);
            }
        }

        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
}

#[allow(dead_code)]
impl<'a> GreeterImpl<'a> {
    async fn say_hello_fanout(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let path = "/hello.Greeter/SayGoodbye".to_string();
        let start = Instant::now();

        let ctx = request.metadata().get_ctx("ctx").unwrap();
        // let graph = self.local_graphs.get(ctx.graph_id()).unwrap();

        log::info!("say_hello_fanout, ddl: {:?}", async_task::get_task_ddl());

        busy_spin(Duration::from_millis(5));

        let mut tasks = Vec::new();
        for idx in 0..2 {
            log::info!("say_hello_fanout, spawning task {}", idx);

            let mut client = self.clients.get(&path).unwrap().clone();
            let mut request = Request::new(HelloRequest {
                name: "SayGoodbye".to_string(),
            });
            self.set_child_ctx(&ctx, &mut request, &path);
            tasks.push(self.executor.spawn(async move {
                client.say_goodbye(request).await.unwrap();
            }));
        }

        for (idx, task) in tasks.into_iter().enumerate() {
            task.await;
            log::info!("say_hello_fanout, completed task {}", idx);
        }

        busy_spin(Duration::from_millis(5));

        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };

        log::info!(
            "say_hello_fanout, elapsed {} ms",
            start.elapsed().as_millis()
        );
        Ok(Response::new(reply))
    }
}

#[derive(Debug)]
pub struct ExecImpl<'a> {
    ex: &'a smol::Executor<'a, AsyncTaskMetadata>,
}

impl<'a> ExecImpl<'a> {
    fn new(ex: &'a smol::Executor<'a, AsyncTaskMetadata>) -> Self {
        Self { ex }
    }

    pub fn spawn<T: Send + 'a>(
        &self,
        future: impl Future<Output = T> + Send + 'a,
    ) -> async_task::Task<T, AsyncTaskMetadata> {
        self.ex.spawn(future)
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
    fn execute(&self, fut: F, ddl: PriorityHint) {
        self.ex.spawn_with_prio(fut, ddl).fallible().detach();
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

    pub async fn get_clients(&self) -> HashMap<Path, GreeterClient<Channel>> {
        let mut clients = HashMap::new();
        for (path, addr) in self.conn_addrs.iter() {
            let client = GreeterClient::connect(addr.clone()).await.unwrap();
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
    static SMOL_EXECUTOR: smol::Executor<'static, AsyncTaskMetadata> = smol::Executor::new();

    init_logging();

    let args = Args::from_args();

    let global_graph = graph::get_global_graph_i2("I2".to_string(), 1_000, 1_000, Some(100), 5_000);

    let mut servers = Vec::new();

    let server1 = {
        let addr: Address = "[::1]:50051".to_string();
        let conn_addrs = HashMap::new();
        let path: Path = "/hello.Greeter/SayGoodbye".to_string();
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

    let server2 = {
        let addr: Address = "[::1]:50052".to_string();
        let mut conn_addrs = HashMap::new();
        conn_addrs.insert(
            "/hello.Greeter/SayGoodbye".to_string() as Path,
            "http://[::1]:50051".to_string() as Address,
        );
        let path: Path = "/hello.Greeter/SayHello".to_string();
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

    let mut handles = Vec::new();

    for i in 0..servers.len() {
        let server = servers[i].clone();

        let ex = Arc::new(ExecImpl::new(&SMOL_EXECUTOR));
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
            let server_ex = ex.clone();

            let greeter = GreeterImpl::new(local_graphs, clients, server_ex);
            log::info!("Listening on {}...", addr);
            Server::builder()
                .add_service(GreeterServer::new(greeter))
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
