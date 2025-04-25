use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::Instant;
use tokio::time::{Duration, interval};

use text_service::text_service_client::TextServiceClient;
use text_service::TextRequest;

use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use csv::WriterBuilder;
use futures::future;
use serde_json;

pub mod text_service {
    tonic::include_proto!("textservice");
}

#[derive(Deserialize, Debug)]
struct LoadConfig {
    rps_list: Vec<u64>,
    duration_per_rps: u64,
    client_port: String,
    output_path: String,
}

fn load_config(file_path: &str) -> Result<LoadConfig, Box<dyn std::error::Error>> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let config: LoadConfig = serde_json::from_reader(reader)?;
    Ok(config)
}

async fn run_load_test(
    rps: u64,
    duration: u64,
    client: Arc<TextServiceClient<tonic::transport::Channel>>,
    csv_writer: tokio::sync::mpsc::UnboundedSender<[u64; 4]>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Track latency and goodput by atomic clones
    let total_latency = Arc::new(AtomicUsize::new(0));
    let goodput = Arc::new(AtomicUsize::new(0));
    let goodput_clone = goodput.clone();
    let total_latency_clone = total_latency.clone();
    let csv_writer_clone = csv_writer.clone();

    // Spawn a task to print goodput and latency
    let metric_handle = tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(1));
        for sec in 0..duration {
            tick.tick().await;
            let count = goodput_clone.swap(0, Ordering::Relaxed) as u64;
            let lat_us = total_latency_clone.swap(0, Ordering::Relaxed) as u64;
            let avg_latency = if count > 0 { lat_us / (rps * duration) } else { 0 };
            println!("Goodput: {} req/s, Avg Latency: {} us", count, avg_latency);

            // Send metrics to CSV writer
            if let Err(_) = csv_writer_clone.send([sec, rps, count, avg_latency]) {
                eprintln!("Failed to send metrics to CSV writer");
            }
        }
    });

    // Request with pace
    let mut pace = interval(Duration::from_secs_f64(1.0 / rps as f64));
    let mut handles = Vec::with_capacity((rps * duration) as usize);
    for _ in 0..rps*duration {
        pace.tick().await;
        let mut txtsvc_client = (*client).clone();
        let goodput = goodput.clone();
        let total_latency = total_latency.clone();

        let handle = tokio::spawn(async move {
            let start = Instant::now();
            let request = tonic::Request::new(TextRequest {
                text: "@john @bob Hello! Check https://openai.com".to_string(),
            });

            if let Ok(_) = txtsvc_client.compose_text(request).await {
                goodput.fetch_add(1, Ordering::Relaxed);
                total_latency.fetch_add(start.elapsed().as_micros() as usize, Ordering::Relaxed);
            }
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    future::join_all(handles).await;
    if let Err(e) = metric_handle.await {
        eprintln!("Metric task failed: {:?}", e);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration from JSON file
    let config = load_config("/home/jiexiao/research/masa-internal/apps/socialnet/src/text_service/benchmark/config.json")?;

    let rps_list: Vec<u64> = config.rps_list; // list of requests per second
    let duration : u64 = config.duration_per_rps; // seconds
    let client_port = config.client_port.clone(); // client port
    let output_path = config.output_path.clone(); // output path

    // Initialize client with atomic RC
    let client = Arc::new(TextServiceClient::connect(client_port).await?);

    // mpsc channle for writing to csv
    let (csv_writer_tx, mut csv_writer_rx) = tokio::sync::mpsc::unbounded_channel::<[u64; 4]>();
    let writer_handle = tokio::spawn(async move {
        let mut csv_writer = WriterBuilder::new()
            .has_headers(true)
            .from_path(output_path)
            .expect("Failed to create CSV writer");
        csv_writer.write_record(&["sec", "rps", "goodput", "avg_latency"]).expect("Failed to write header");
        while let Some([sec, rps, gp, lat]) = csv_writer_rx.recv().await {
            csv_writer.write_record(&[sec,rps,gp,lat].map(|v| v.to_string())).unwrap();
        }
        csv_writer.flush().unwrap();
    });

    for &rps in &rps_list {
        println!("Running load test with RPS: {}, Duration: {}, Total Requests: {}", rps, duration, rps * duration);
        run_load_test(rps, duration, client.clone(), csv_writer_tx.clone()).await?;
    }
    drop(csv_writer_tx);
    writer_handle.await.unwrap();

    // Wait for all tasks to complete
    println!("Load test completed.");
    Ok(())

}