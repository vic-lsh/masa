use hello::greeter_client::GreeterClient;
use hello::greeter_server::{Greeter, GreeterServer};
use hello::{HelloReply, HelloRequest};
use hyper::rt::{Exec, Executor};
use rand_distr::{Distribution, Normal};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
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

pub struct GreeterImpl {}

impl Default for GreeterImpl {
    fn default() -> Self {
        Self {}
    }
}

fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
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

    busy_spin(Duration::from_millis(std::cmp::max(1, random_value as u64)));
}

#[tonic::async_trait]
impl Greeter for GreeterImpl {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let mean_ms = 10;
        let std_ms = 0;
        rand_busy_spin(mean_ms, std_ms).await;

        let reply = hello::HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_hola(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let mean_ms = 10;
        let std_ms = 0;
        rand_busy_spin(mean_ms, std_ms).await;

        let mut client = GreeterClient::connect("http://[::1]:50053").await.unwrap();
        let reply = client.say_hello(request).await.unwrap();
        Ok(reply)
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
        Arc::new(ExecImpl::new(smol_ex.clone(), 1)),
        Arc::new(ExecImpl::new(smol_ex.clone(), 1)),
    ];

    println!("Spawning {} server threads...", args.num_threads);
    for _ in 0..args.num_threads {
        // [NOTE] exs use the same smol::Executor instance.
        let ex = exs[0].clone();

        // [NOTE] Semantically, it is equivalent to tokio::spawn(ex_clone.run()).
        // However, we use std::thread::spawn() to have dedicated threads for
        // executors that poll futures based on deadline hints.
        std::thread::spawn(move || {
            // [DEBUG] It will not work if args.num_threads is 1.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(ex.run());

            // future::block_on(ex.run());
        });
    }

    let addrs = [
        "[::1]:50051".parse().unwrap(),
        "[::1]:50052".parse().unwrap(),
        "[::1]:50053".parse().unwrap(),
    ];
    let mut handles = Vec::new();
    for i in 0..addrs.len() {
        let ex = exs[i].clone();
        let addr = addrs[i];
        let h = tokio::spawn(async move {
            let greeter = GreeterImpl::default();
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
