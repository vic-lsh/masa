use std::sync::Arc;
use std::time::{Duration, Instant};

use hyper::rt::Exec;
use tonic::{transport::Server, Request, Response, Status};

use hello_world::greeter_server::{Greeter, GreeterServer};
use hello_world::{HelloReply, HelloRequest};

use futures_lite::future;

use rand::prelude::*;
use rand_distr::{Distribution, Normal};

use hyper::rt::{DeadlineHint, Executor};

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
        let stddev_ms = 0;
        rand_busy_spin(mean_ms, stddev_ms);

        let reply = hello_world::HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
}

#[derive(Debug)]
struct MyExec<'a> {
    ex: smol::Executor<'a>,
}

impl<'a> MyExec<'a> {
    fn new() -> Self {
        Self {
            ex: smol::Executor::new(),
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

impl<'a, F> Executor<F> for MyExec<'a>
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
    const RT_THREAD_COUNT: usize = 16;

    let addr = "[::1]:50051".parse().unwrap();
    let greeter = MyGreeter::default();

    // [TODO:Rivers]
    // Change DeadlineHint::Background to DeadlineHint::Infra;
    // let smol_exec = Arc::new(smol::Executor::new());
    // let hi_exec = MyExec::new(smol_exec.clone(), DeadlineHint::Some(1));
    // let low_exec = MyExec::new(smol_exec.clone(), DeadlineHint::Some(2));

    println!("GreeterServer listening on {}", addr);

    let ex = Arc::new(MyExec::new());

    for _ in 0..RT_THREAD_COUNT {
        let ex_clone = ex.clone();
        // [NOTE] Semantically, it is equivalent to tokio::spawn(ex_clone.run()).
        // However, we use std::thread::spawn() to have dedicated threads for
        // executors that poll futures based on deadline hints.
        std::thread::spawn(move || future::block_on(ex_clone.run()));
    }

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve_with_executor(addr, Exec::Executor(ex))
        .await?;

    Ok(())
}
