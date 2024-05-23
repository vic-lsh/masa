use std::sync::Arc;
use std::time::{Duration, Instant};

use hyper::rt::Exec;
use tonic::{transport::Server, Request, Response, Status};

use hello_world::greeter_server::{Greeter, GreeterServer};
use hello_world::{HelloReply, HelloRequest};

use futures_lite::future;
use smol::Executor;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

pub struct MyGreeter {
    x: u32,
}

impl Default for MyGreeter {
    fn default() -> Self {
        Self { x: 32 }
    }
}

fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        //busy_spin(Duration::from_millis(500));

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
        loop {
            self.ex.tick().await;
        }
    }
}

impl<'a, F> hyper::rt::Executor<F> for LocalExec<'a>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, fut: F) {
        self.ex.spawn(fut).fallible().detach();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse().unwrap();
    let greeter = MyGreeter::default();

    println!("GreeterServer listening on {}", addr);

    let ex = Arc::new(LocalExec::new());
    let ex_clone = ex.clone();
    std::thread::spawn(move || future::block_on(ex_clone.run()));

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve_with_executor(addr, Exec::Executor(ex))
        .await?;

    Ok(())
}
