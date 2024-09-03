use futures_lite::future;
use hello::greeter_server::{Greeter, GreeterServer};
use hello::{HelloReply, HelloRequest};
use hyper::rt::{Exec, Executor};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
use tonic::{transport::Server, Request, Response, Status};
use tonic_masa::DeadlineHint;

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
            // [TODO] Change to tokio yield.
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

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Server for benchmarking")]
pub struct Args {
    #[structopt(short, long, default_value = "1")]
    pub num_threads: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Hotel {
    name: String,
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

#[tonic::async_trait]
impl Greeter for GreeterImpl {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        busy_spin(Duration::from_millis(10));

        let memcache = memcache::Client::with_pool_size("memcache://127.0.0.1:11003", 32).unwrap();
        memcache.flush().unwrap();
        memcache.set("Reboot", "ing...", 0).unwrap();
        let value = memcache.get::<String>("Reboot");
        println!("memcached: {:?}", value);

        let db_client = mongodb::Client::with_uri_str("mongodb://127.0.0.1:27003")
            .await
            .unwrap();
        let db = db_client.database("reboot");
        let cl = db.collection::<Hotel>("reboot");
        cl.delete_many(mongodb::bson::doc! {}, None).await.unwrap();
        let hotel = Hotel {
            name: "Reboot".to_string(),
        };
        cl.insert_one(hotel, None).await.unwrap();
        let value = cl.find_one(mongodb::bson::doc! {}, None).await.unwrap();
        println!("mongodb: {:?}", value);

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

    async fn say_goodbye(
        &self,
        _request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        panic!("Not implemented");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();

    let ex = Arc::new(ExecImpl::new(Arc::new(smol::Executor::new())));
    for _ in 0..args.num_threads {
        let ex = ex.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(ex.run());
        });
    }

    let addr = "[::1]:50051".parse().unwrap();
    let mut handles = Vec::new();
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

    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}
