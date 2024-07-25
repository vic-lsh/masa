use hello::{greeter_client::GreeterClient, HelloRequest};
use std::collections::HashMap;
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use structopt::StructOpt;
use tonic::metadata::{Context, GlobalGraph, LocalGraph, Path, Span};
use tonic::transport::Channel;

mod graph;

pub mod hello {
    tonic::include_proto!("hello");
}
#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
    #[structopt(short, long, default_value = "http://[::1]:50051")]
    pub addr: String,
}

#[derive(Debug)]
struct LoadGenerator {
    global_graph: GlobalGraph,
    client: GreeterClient<Channel>,
    concurrency: usize,
    rps_cnt: Arc<AtomicUsize>,
}

impl LoadGenerator {
    pub fn new(
        global_graph: GlobalGraph,
        client: GreeterClient<Channel>,
        concurrency: usize,
        rps_cnt: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            global_graph,
            client,
            concurrency,
            rps_cnt,
        }
    }

    pub async fn run(&self) -> Result<(), Box<dyn Error>> {
        let mut handles = Vec::with_capacity(self.concurrency);

        for _ in 0..self.concurrency {
            let gid = self.global_graph.gid().clone();
            let local_graph = self.global_graph.get_source().clone();

            let rps_cnt = self.rps_cnt.clone();
            let mut client = self.client.clone();
            let request = HelloRequest {
                name: "Tonic".into(),
            };

            let h = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    rps_cnt.fetch_add(1, Ordering::Relaxed);

                    let ctx = Context::new(gid.clone(), 1, 100, Some(local_graph.clone()));
                    let mut request = tonic::Request::new(request.clone());
                    request.metadata_mut().insert_ctx("par_ctx", &ctx);

                    let _response = client.say_hello(request).await.unwrap();
                }
            });
            handles.push(h);
        }

        for h in handles {
            h.await.unwrap();
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();

    let rps_cnt = Arc::new(AtomicUsize::new(0));
    let rps_cnt_clone = rps_cnt.clone();
    tokio::spawn(async move {
        let mut prev = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let now = rps_cnt_clone.load(Ordering::Relaxed);
            let rps = now - prev;
            println!("rps: {}", rps);
            prev = now;
        }
    });

    let load_gen = {
        let global_graph = graph::get_global_graph();

        let client = GreeterClient::connect(args.addr).await?;

        let load_gen = LoadGenerator::new(global_graph, client, 1, rps_cnt);
        load_gen
    };
    load_gen.run().await.unwrap();

    Ok(())
}
