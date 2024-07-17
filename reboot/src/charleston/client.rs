use hello::greeter_client::GreeterClient;
use hello::HelloRequest;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use structopt::StructOpt;
use tonic::metadata::MasaContext;

pub mod hello {
    tonic::include_proto!("hello");
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
    #[structopt(short, long, default_value = "http://[::1]:50051")]
    pub addr: String,
}

async fn loadgen(
    addr: String,
    rpc_count: Arc<AtomicUsize>,
) -> Result<(), Box<dyn std::error::Error>> {
    // [TODO] Pass concurrency.
    let concurrency = 32;
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
                let mut request = tonic::Request::new(request.clone());
                let metadata = request.metadata_mut();
                let par_ctx = MasaContext::default();
                metadata.insert_ctx("par_ctx", par_ctx);
                let _ = client.say_hello(request).await.unwrap();
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
    let h = tokio::spawn(async move {
        loadgen(args.addr, c).await.unwrap();
    });

    h.await.unwrap();

    Ok(())
}
