use futures_lite::future;
use hello::greeter_server::{Greeter, GreeterServer};
use hello::{HelloReply, HelloRequest};
use hyper::rt::{Exec, Executor};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
use tonic::metadata::{GlobalGraph, LocalGraph, Path, Span};
use tonic::{transport::Server, Request, Response, Status};
use tonic_deadline::DeadlineHint;

pub mod hello {
    tonic::include_proto!("hello");
}

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
    pub num_threads: usize,
}

pub struct GreeterImpl {
    local_graphs: HashMap<Path, LocalGraph>,
}

impl GreeterImpl {
    pub fn new(local_graphs: HashMap<Path, LocalGraph>) -> Self {
        Self { local_graphs }
    }
}

fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
}

#[tonic::async_trait]
impl Greeter for GreeterImpl {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.gid()).unwrap();
        ctx.set_local_graph(local_graph.clone());

        let mean_ms = 2;
        busy_spin(Duration::from_millis(mean_ms));

        let reply = hello::HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_hola(
        &self,
        _request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        panic!("Not implemented");
    }
}

#[derive(Debug)]
struct ExecImpl<'a> {
    ex: Arc<smol::Executor<'a>>,
    ddl: u64,
    start_at: u64,
}

impl<'a> ExecImpl<'a> {
    fn new(ex: Arc<smol::Executor<'a>>, ddl: u64) -> Self {
        Self {
            ex,
            ddl,
            start_at: time_now(),
        }
    }

    async fn run(&self) {
        // [NOTE] Two-level queues are used in smol::Executor::run().
        // self.ex
        //     .run(async {
        //         loop {
        //             future::yield_now().await;
        //         }
        //     })
        //     .await;

        // [NOTE] Only a global queue is used in smol::Executor::tick().
        loop {
            self.ex.tick().await;
            // [NOTE] Yield to the tokio runtime.
            future::yield_now().await;
        }
    }
}

impl<'a, F> Executor<F> for ExecImpl<'a>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, fut: F, _ddl: DeadlineHint) {
        let ddl = DeadlineHint::new(time_now() - self.start_at + self.ddl);
        self.ex.spawn_with_ddl(fut, ddl).fallible().detach();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();

    let smol_ex = Arc::new(smol::Executor::new());
    let exs = [
        Arc::new(ExecImpl::new(smol_ex.clone(), 1)),
        // Arc::new(ExecImpl::new(smol_ex.clone(), 1)),
    ];

    eprintln!("Spawning {} server threads...", args.num_threads);
    for _ in 0..args.num_threads {
        // [NOTE] exs use the same smol::Executor instance.
        let ex = exs[0].clone();

        // [NOTE] Semantically, it is equivalent to tokio::spawn(ex_clone.run()).
        // However, we use std::thread::spawn() to have dedicated threads for
        // executors that poll futures based on deadline hints.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(ex.run());

            // future::block_on(ex.run());
        });
    }

    let mut local_graphs = HashMap::new();
    local_graphs.insert(
        "Source".to_string() as Path,
        LocalGraph::new(vec![Span::new("/hello.Greeter/SayHello".to_string(), 1, 1)]),
    );
    local_graphs.insert(
        "/hello.Greeter/SayHello".to_string() as Path,
        LocalGraph::new(vec![
            Span::new("Head".to_string(), 1, 1),
            Span::new("Tail".to_string(), 1, 1),
        ]),
    );
    let global_graph = GlobalGraph::new("GID".to_string(), local_graphs);

    let addrs = [
        "[::1]:50051".parse().unwrap(),
        // "[::1]:50052".parse().unwrap(),
    ];
    let mut handles = Vec::new();

    for i in 0..addrs.len() {
        let ex = exs[i].clone();
        let addr = addrs[i];
        let global_graph = global_graph.clone();

        let h = tokio::spawn(async move {
            let mut local_graphs = HashMap::new();
            let path: Path = "/hello.Greeter/SayHello".to_string();
            local_graphs.insert(
                global_graph.gid().clone(),
                global_graph.get_local_graph(&path).clone(),
            );
            let greeter = GreeterImpl::new(local_graphs);

            eprintln!("Listening on {}...", addr);
            Server::builder()
                .add_service(GreeterServer::new(greeter))
                .serve_with_executor(addr, Exec::Executor(ex))
                .await
                .unwrap();
        });

        handles.push(h);
    }

    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}
