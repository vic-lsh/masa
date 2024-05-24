use std::sync::Arc;
use std::time::{Duration, Instant};

use hyper::rt::Exec;
use tonic::{transport::Server, Request, Response, Status};

use hello_world::greeter_server::{Greeter, GreeterServer};
use hello_world::{HelloReply, HelloRequest};

use futures_lite::future;
use smol::Executor;

use rand::prelude::*;
use rand_distr::{Distribution, Normal};

use hyper::rt::{self, DeadlineHint};

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

pub struct MyGreeter {}

impl Default for MyGreeter {
    fn default() -> Self {
        Self {}
    }
}

fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
}

fn rand_busy_spin(mean_ms: impl Into<f64>, stddev_ms: impl Into<f64>) {
    let mut rng = thread_rng();

    let mean = mean_ms.into();
    let std_dev = stddev_ms.into();
    let normal = Normal::new(mean, std_dev).unwrap();
    let random_value: f64 = normal.sample(&mut rng);

    busy_spin(Duration::from_millis(std::cmp::min(1, random_value as u64)));
}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let mean_ms = 10;
        let stddev_ms = 3;
        let stddev_ms = 0;
        rand_busy_spin(mean_ms, stddev_ms);

        let reply = hello_world::HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
}

#[derive(Debug)]
struct LocalExec<'a> {
    ex: Executor<'a>,
}

impl<'a> LocalExec<'a> {
    fn new() -> Self {
        Self {
            ex: Executor::new(),
        }
    }

    async fn run(&self) {
        // Two-level queues from smol::Executor::run()
        // self.ex
        //     .run(async {
        //         loop {
        //             future::yield_now().await;
        //         }
        //     })
        //     .await;

        // Global queue only from smol::Executor::tick()
        loop {
            self.ex.tick().await;
        }
    }
}

impl<'a, F> rt::Executor<F> for LocalExec<'a>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, fut: F, ddl: DeadlineHint) {
        self.ex.spawn_with_ddl(fut, ddl).fallible().detach();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    const RT_THREAD_COUNT: usize = 2;

    let addr = "[::1]:50051".parse().unwrap();
    let greeter = MyGreeter::default();

    println!("GreeterServer listening on {}", addr);

    let ex = Arc::new(LocalExec::new());

    for _ in 0..RT_THREAD_COUNT {
        let ex_clone = ex.clone();
        std::thread::spawn(move || future::block_on(ex_clone.run()));
    }

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve_with_executor(addr, Exec::Executor(ex))
        .await?;

    Ok(())
}
