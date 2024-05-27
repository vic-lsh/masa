use futures_lite::future;
use hyper::rt::{DeadlineHint, Exec, Executor};
use rand_distr::{Distribution, Normal};
use std::sync::Arc;
use std::time::{Duration, Instant};
use structopt::StructOpt;
use tonic::{transport::Server, Request, Response, Status};

use async_compat::{Compat, CompatExt};
use hello_world::greeter_server::{Greeter, GreeterServer};
use hello_world::{HelloReply, HelloRequest};

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Server for benchmarking")]
pub struct Args {
    #[structopt(short, long, default_value = "1")]
    pub num_threads: usize,
}

pub struct MyGreeter {}

impl Default for MyGreeter {
    fn default() -> Self {
        Self {}
    }
}

async fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {
        future::yield_now().await;
    }
}

async fn rand_busy_spin(mean_ms: impl Into<f64>, std_ms: impl Into<f64>) {
    let random_value = {
        let mut rng = rand::thread_rng();

        let mean = mean_ms.into();
        let std = std_ms.into();
        let normal = Normal::new(mean, std).unwrap();
        let random_value: f64 = normal.sample(&mut rng);
        random_value
    };

    busy_spin(Duration::from_millis(std::cmp::max(1, random_value as u64))).await;
}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let mean_ms = 10;
        let std_ms = 0;
        rand_busy_spin(mean_ms, std_ms).await;

        let reply = hello_world::HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
}

#[derive(Debug)]
struct ExecImpl<'a> {
    ex: Arc<smol::Executor<'a>>,
    ddl: DeadlineHint,
}

impl<'a> ExecImpl<'a> {
    fn new(ex: Arc<smol::Executor<'a>>, ddl: DeadlineHint) -> Self {
        Self { ex, ddl }
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
        }
    }
}

impl<'a, F> Executor<F> for ExecImpl<'a>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, fut: F, _ddl: DeadlineHint) {
        self.ex
            .spawn_with_ddl(Compat::new(fut), self.ddl.clone())
            .fallible()
            .detach();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();

    let smol_ex = Arc::new(smol::Executor::new());
    let exs = [
        Arc::new(ExecImpl::new(smol_ex.clone(), DeadlineHint::new(1))),
        Arc::new(ExecImpl::new(smol_ex.clone(), DeadlineHint::new(2))),
    ];

    println!("spawning {} server threads", args.num_threads);
    for _ in 0..args.num_threads {
        let ex = exs[0].clone();
        // [NOTE] Semantically, it is equivalent to tokio::spawn(ex_clone.run()).
        // However, we use std::thread::spawn() to have dedicated threads for
        // executors that poll futures based on deadline hints.
        std::thread::spawn(move || future::block_on(ex.run()));
    }

    let addrs = [
        "[::1]:50051".parse().unwrap(),
        "[::1]:50052".parse().unwrap(),
    ];
    let mut handles = Vec::new();
    for i in 0..2 {
        let ex = exs[i].clone();
        let addr = addrs[i];
        let h = tokio::spawn(async move {
            let greeter = MyGreeter::default();
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
