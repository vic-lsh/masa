use hello::greeter_client::GreeterClient;
use hello::HelloRequest;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use structopt::StructOpt;
use tonic::metadata::{Context, GlobalGraph, LocalGraph, Span};

pub mod hello {
    tonic::include_proto!("hello");
}
#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
    #[structopt(short, long, default_value = "http://[::1]:50051")]
    pub addr: String,
}

async fn load_gen(
    addr: String,
    concurrency: usize,
    rps_cnt: Arc<AtomicUsize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut handles = Vec::with_capacity(concurrency);

    let mut local_graphs = HashMap::new();
    local_graphs.insert(
        "Source".to_string(),
        LocalGraph::new(vec![Span::new("/hello.Greeter/SayHello".to_string(), 1, 1)]),
    );
    local_graphs.insert(
        "/hello.Greeter/SayHello".to_string(),
        LocalGraph::new(vec![
            Span::new("Head".to_string(), 1, 1),
            Span::new("Tail".to_string(), 1, 1),
        ]),
    );
    let global_graph = GlobalGraph::new("GID".to_string(), local_graphs);

    let client = GreeterClient::connect(addr).await?;
    for _ in 0..concurrency {
        let gid = global_graph.gid().clone();
        let local_graph = global_graph.get_source().clone();

        let rps_cnt = rps_cnt.clone();
        let mut client = client.clone();
        let request = HelloRequest {
            name: "Tonic".into(),
        };

        let h = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                rps_cnt.fetch_add(1, Ordering::Relaxed);

                let ctx = Context::new(gid.clone(), 1, 10, Some(local_graph.clone()));

                let mut request = tonic::Request::new(request.clone());
                request.metadata_mut().insert_ctx("par_ctx", &ctx);

                let response = client.say_hello(request).await.unwrap();
                // let child_ctx = response.metadata().get_ctx("ctx").unwrap();

                let ctx = response.metadata().get_ctx("par_ctx").unwrap();
                println!("[client] ctx: {:?}", ctx);
            }
        });
        handles.push(h);
    }

    for h in handles {
        h.await.unwrap();
    }
    Ok(())
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

    let rps_cnt_clone = rps_cnt.clone();
    let h = tokio::spawn(async move {
        load_gen(args.addr, 1, rps_cnt_clone).await.unwrap();
    });

    h.await.unwrap();

    Ok(())
}
