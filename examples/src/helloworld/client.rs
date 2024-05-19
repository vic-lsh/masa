use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hello_world::greeter_client::GreeterClient;
use hello_world::HelloRequest;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

async fn loadgen() -> Result<(), Box<dyn std::error::Error>> {
    let client = GreeterClient::connect("http://[::1]:50051").await?;

    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    tokio::spawn(async move {
        let mut prev = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let c_curr = c.load(Ordering::Relaxed);
            let rps = c_curr - prev;
            println!("rps: {}", rps);
            prev = c_curr;
        }
    });

    let num_clients = 256; // tunable
    let mut handles = Vec::with_capacity(num_clients);
    for _ in 0..num_clients {
        let mut cl = client.clone();
        let cnt = counter.clone();
        let h = tokio::spawn(async move {
            let request = HelloRequest {
                name: "Tonic".into(),
            };
            loop {
                let _ = cl
                    .say_hello(tonic::Request::new(request.clone()))
                    .await
                    .unwrap();
                cnt.fetch_add(1, Ordering::Relaxed);
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
    loadgen().await
}
