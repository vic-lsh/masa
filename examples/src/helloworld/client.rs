use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use structopt::StructOpt;

use hello_world::greeter_client::GreeterClient;
use hello_world::HelloRequest;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
    #[structopt(short, long, default_value = "http://[::1]:50051")]
    pub addr1: String,
    // #[structopt(short, long, default_value = "http://[::1]:50052")]
    // pub addr2: String,
}

async fn loadgen(
    addr: String,
    rpc_count: Arc<AtomicUsize>,
) -> Result<(), Box<dyn std::error::Error>> {
    // [TODO] Pass concurrency.
    let concurrency = 1;
    let mut handles = Vec::with_capacity(concurrency);

    let client = GreeterClient::connect(addr).await?;
    for _ in 0..concurrency {
        let mut client = client.clone();
        let c = rpc_count.clone();
        let h = tokio::spawn(async move {
            let request = HelloRequest {
                name: "Tonic".into(),
            };
            loop {
                let _ = client
                    .say_hello(tonic::Request::new(request.clone()))
                    .await
                    .unwrap();
                c.fetch_add(1, Ordering::Relaxed);
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

    let cnt = Arc::new(AtomicUsize::new(0));
    let c = cnt.clone();
    tokio::spawn(async move {
        let mut prev = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let now = c.load(Ordering::Relaxed);
            let rps = now - prev;
            println!("rps: {}", rps);
            prev = now;
        }
    });

    let c = cnt.clone();
    let h1 = tokio::spawn(async move {
        loadgen(args.addr1, c).await.unwrap();
    });
    // let c = cnt.clone();
    // let h2 = tokio::spawn(async move {
    //     loadgen(args.addr2, c).await.unwrap();
    // });

    h1.await.unwrap();
    // h2.await.unwrap();

    Ok(())
}
