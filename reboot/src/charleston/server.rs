pub mod hello {
    tonic::include_proto!("hello");
}

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use env_logger::{Builder, Env};
use futures_lite::future;
use structopt::StructOpt;

use hyper::rt::{Exec, Executor};
use tonic::{
    masa::AsyncTaskMetadata,
    transport::{Channel, Server},
    Request, Response, Status,
};
use tonic_masa::{Address, MethodId, PriorityHint};

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
    clients: HashMap<MethodId, GreeterClient<Channel>>,
    executor: Arc<ExecImpl<'a>>,
}

impl<'a> GreeterImpl<'a> {
    pub fn new(
        clients: HashMap<MethodId, GreeterClient<Channel>>,
        executor: Arc<ExecImpl<'a>>,
    ) -> Self {
        Self { clients, executor }
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
        log::info!("say_goodbye, ddl: {:?}", async_task::get_task_ddl());

        // [CL] Compute spans are not supported for now.

        // let ctx = request.metadata().get_ctx("ctx").unwrap();
        // let graph = self.local_graphs.get(ctx.graph_id()).unwrap();

        // let spans = graph.spans();
        // assert!(spans.len() == 2);

        // for i in 0..spans.len() {
        //     let span = &spans[i];
        //     let path = span.path();

        //     if i == 0 || i == spans.len() - 1 {
        //         let elapse = span.distribution().sample(ctx.request_id());
        //         let start_at = time_now();
        //         busy_spin(Duration::from_micros(elapse));
        //         let latency = time_now() - start_at;
        //         self.track_span(&ctx, path, latency);
        //     }
        // }

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
        log::info!("say_hello_fanout, ddl: {:?}", async_task::get_task_ddl());

        let method_id: MethodId = "/hello.Greeter/SayGoodbye".to_string();
        let start = Instant::now();

        // let ctx = request.metadata().get_ctx("ctx").unwrap();
        // let graph = self.local_graphs.get(ctx.graph_id()).unwrap();

        busy_spin(Duration::from_millis(5));

        let mut tasks = Vec::new();
        for idx in 0..2 {
            log::info!("say_hello_fanout, spawning task: {}", idx);

            let mut client = self.clients.get(&method_id).unwrap().clone();
            let request = Request::new(HelloRequest {
                name: "SayGoodbye".to_string(),
            });
            request.metadata_mut().insert_ctx("par_ctx", &ctx);
            tasks.push(async_executor::spawn(async move {
                client.say_goodbye(request).await.unwrap();
            }));
            // tasks.push(self.executor.spawn(async move {
            //     client.say_goodbye(request).await.unwrap();
            // }));
        }

        for (idx, task) in tasks.into_iter().enumerate() {
            task.await;
            log::info!("say_hello_fanout, completed task: {}", idx);
        }

        busy_spin(Duration::from_millis(5));

        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };

        log::info!(
            "say_hello_fanout, elapsed: {} us",
            start.elapsed().as_micros()
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

    // pub fn spawn<T: Send + 'a>(
    //     &self,
    //     future: impl Future<Output = T> + Send + 'a,
    // ) -> async_task::Task<T> {
    //     async_executor::spawn(future)
    //     // self.ex.spawn(future)
    // }

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
        async_executor::spawn_with_ddl(fut, ddl).fallible().detach();
    }
}

#[derive(Debug, Clone)]
struct VirtualServer {
    addr: Address,
    conn_addrs: HashMap<MethodId, Address>,
    n_threads: usize,
}

impl VirtualServer {
    pub fn new(addr: Address, conn_addrs: HashMap<MethodId, Address>, n_threads: usize) -> Self {
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
            n_threads,
        }
    }

    pub fn addr(&self) -> &Address {
        &self.addr
    }

    pub async fn get_clients(&self) -> HashMap<MethodId, GreeterClient<Channel>> {
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

    let mut servers = Vec::new();

    let server1 = {
        let addr: Address = "[::1]:50051".to_string();
        let conn_addrs = HashMap::new();
        let server = VirtualServer::new(addr, conn_addrs, args.n_threads);
        server
    };
    servers.push(server1);

    let server2 = {
        let addr: Address = "[::1]:50052".to_string();
        let mut conn_addrs = HashMap::new();
        conn_addrs.insert(
            "/hello.Greeter/SayGoodbye".to_string() as MethodId,
            "http://[::1]:50051".to_string() as Address,
        );
        let server = VirtualServer::new(addr, conn_addrs, args.n_threads);
        server
    };
    servers.push(server2);

    let mut handles = Vec::new();

    for i in 0..servers.len() {
        let server = servers[i].clone();

        let ex = Arc::new(ExecImpl::new(&SMOL_EXECUTOR));
        // for _ in 0..server.n_threads() {
        //     let ex = ex.clone();
        //     std::thread::spawn(move || {
        //         let rt = tokio::runtime::Builder::new_current_thread()
        //             .enable_all()
        //             .build()
        //             .unwrap();
        //         rt.block_on(ex.run());
        //     });
        // }

        let h = tokio::spawn(async move {
            let addr = server.addr().parse().unwrap();
            let clients = server.get_clients().await;
            let server_ex = ex.clone();

            let greeter = GreeterImpl::new(clients, server_ex);
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
