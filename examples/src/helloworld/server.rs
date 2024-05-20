use std::sync::Arc;

use hyper::rt::Exec;
use tonic::{transport::Server, Request, Response, Status};

use hello_world::greeter_server::{Greeter, GreeterServer};
use hello_world::{HelloReply, HelloRequest};

use async_executor::Executor;
use futures_lite::future;

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

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        // println!("Got a request from {:?}", request.remote_addr());

        let reply = hello_world::HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
}

// Since the Server needs to spawn some background tasks, we needed
// to configure an Executor that can spawn !Send futures...
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
        let task = self.ex.spawn(fut);
        // [TODO] Fix this.
        std::mem::forget(task);
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
