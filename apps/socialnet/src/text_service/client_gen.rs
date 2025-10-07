use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep_until;
use tokio::time::Instant as TokioInstant;

use text_service::text_service_client::TextServiceClient;
use text_service::TextRequest;

use csv::WriterBuilder;
use futures::future;
use rand::prelude::*;
use rand::SeedableRng;
use rand_distr::{Distribution, Zipf};
use serde::Deserialize;
use serde_json;
use std::fs::File;
use std::io::BufReader;
use rand::prelude::*;
use rand::SeedableRng;
use rand_distr::{Zipf, Distribution};

pub mod text_service {
    tonic::include_proto!("textservice");
}

#[derive(Deserialize, Debug)]
struct LoadConfig {
    rps_list: Vec<u64>,
    duration_per_rps: u64,
    client_port: String,
    output_path: String,
    seed: u64,
    seed: u64,
}

fn load_config(file_path: &str) -> Result<LoadConfig, Box<dyn std::error::Error>> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let config: LoadConfig = serde_json::from_reader(reader)?;
    Ok(config)
}

fn generate_url(index: u64, seed: u64) -> String {
    const DOMAINS: [&str; 5] = ["openai", "rustlang", "example", "coolapp", "techblog"];
    const TLDS: [&str; 5] = ["com", "org", "net", "io", "dev"];
    const PATHS: [&str; 6] = ["home", "about", "login", "user", "docs", "contact"];

    let mut rng = StdRng::seed_from_u64(seed + index);
    let zipf_d = Zipf::new(DOMAINS.len() as u64, 1.2).unwrap();
    let zipf_t = Zipf::new(TLDS.len() as u64, 1.2).unwrap();
    let zipf_p = Zipf::new(PATHS.len() as u64, 1.1).unwrap();

    let d = DOMAINS[zipf_d.sample(&mut rng) as usize - 1];
    let t = TLDS[zipf_t.sample(&mut rng) as usize - 1];
    let p = PATHS[zipf_p.sample(&mut rng) as usize - 1];
    let id = index % 100;

    format!("https://{}.{}.{}.{}/{}", d, id, d, t, p)
}

fn generate_url(index: u64, seed: u64) -> String {
    const DOMAINS: [&str; 5] = ["openai", "rustlang", "example", "coolapp", "techblog"];
    const TLDS: [&str; 5] = ["com", "org", "net", "io", "dev"];
    const PATHS: [&str; 6] = ["home", "about", "login", "user", "docs", "contact"];

    let mut rng = StdRng::seed_from_u64(seed + index);
    let zipf_d = Zipf::new(DOMAINS.len() as u64, 1.2).unwrap();
    let zipf_t = Zipf::new(TLDS.len() as u64, 1.2).unwrap();
    let zipf_p = Zipf::new(PATHS.len() as u64, 1.1).unwrap();

    let d = DOMAINS[zipf_d.sample(&mut rng) as usize - 1];
    let t = TLDS[zipf_t.sample(&mut rng) as usize - 1];
    let p = PATHS[zipf_p.sample(&mut rng) as usize - 1];
    let id = index % 100;

    format!("https://{}.{}.{}.{}/{}", d, id, d, t, p)
}

async fn run_load_test(
    rps: u64,
    duration: u64,
    client: Arc<TextServiceClient<tonic::transport::Channel>>,
    csv_writer: tokio::sync::mpsc::UnboundedSender<[u64; 4]>,
    seed: u64,
    seed: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Request with pace
    let mut handles = Vec::with_capacity((rps * duration) as usize);
    let start_time = TokioInstant::now();
    for i in 0..rps * duration {
    let start_time = TokioInstant::now();
    for i in 0..rps * duration {
        let mut txtsvc_client = (*client).clone();
        let csv_writer = csv_writer.clone();
        let delay_ns = (1_000_000_000f64 / rps as f64) * i as f64;
        let scheduled_time = start_time + Duration::from_nanos(delay_ns as u64);
        let csv_writer = csv_writer.clone();
        let delay_ns = (1_000_000_000f64 / rps as f64) * i as f64;
        let scheduled_time = start_time + Duration::from_nanos(delay_ns as u64);

        let handle = tokio::spawn(async move {
            sleep_until(scheduled_time).await;
            let mut rng = StdRng::seed_from_u64(seed + i);
            let zipf = Zipf::new(2000, 1.03).unwrap(); // moderate skew to simulate hot spots
            let u1 = zipf.sample(&mut rng) as u64;
            let u2 = zipf.sample(&mut rng) as u64;
            let url = generate_url(i, seed);

            let start = Instant::now();
            let request = tonic::Request::new(TextRequest {
                text: format!("@user{} @user{} Hello! Check {}", u1, u2, url),
                text: format!("@user{} @user{} Hello! Check {}", u1, u2, url),
            });

            if let Ok(_) = txtsvc_client.compose_text(request).await {
                let elapsed = start.elapsed().as_micros() as u64;
                let is_good: u64 = {
                    if elapsed < 50000 {
                        // 50ms
                        1
                    } else {
                        0
                    }
                };
                let _ = csv_writer.send([rps, i, is_good, elapsed]);
                if i % 1000 == 0 {
                    println!("Request {} completed", i);
                }
            }
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    future::join_all(handles).await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration from JSON file
    let config = load_config("/home/jiexiao/research/masa-internal/apps/socialnet/src/text_service/benchmark/config.json")?;

    let rps_list: Vec<u64> = config.rps_list; // list of requests per second
    let duration: u64 = config.duration_per_rps; // seconds
    let client_port = config.client_port.clone(); // client port
    let output_path = config.output_path.clone(); // output path
    let seed: u64 = config.seed; // Seed for random number generation
    let seed: u64 = config.seed; // Seed for random number generation

    // Initialize client with atomic RC
    let client = Arc::new(TextServiceClient::connect(client_port).await?);

    // mpsc channle for writing to csv
    let (csv_writer_tx, mut csv_writer_rx) = tokio::sync::mpsc::unbounded_channel::<[u64; 4]>();
    let writer_handle = tokio::spawn(async move {
        let mut csv_writer = WriterBuilder::new()
            .has_headers(true)
            .from_path(output_path)
            .expect("Failed to create CSV writer");
        csv_writer
            .write_record(&["rps", "index", "is_good", "latency"])
            .expect("Failed to write header");
        while let Some([rps, i, is_good, lat]) = csv_writer_rx.recv().await {
        while let Some([rps, i, is_good, lat]) = csv_writer_rx.recv().await {
            csv_writer
                .write_record(&[rps, i, is_good, lat].map(|v| v.to_string()))
                .write_record(&[rps, i, is_good, lat].map(|v| v.to_string()))
                .unwrap();
        }
        csv_writer.flush().unwrap();
    });

    for &rps in &rps_list {
        println!(
            "Running load test with RPS: {}, Duration: {}, Total Requests: {}",
            rps,
            duration,
            rps * duration
        );
        run_load_test(rps, duration, client.clone(), csv_writer_tx.clone(), seed).await?;

        // Sleep for a while to avoid overwhelming the server
        let sleep_duration = Duration::from_secs(5);
        println!("Sleeping for {:?}", sleep_duration);
        tokio::time::sleep(sleep_duration).await;
        run_load_test(rps, duration, client.clone(), csv_writer_tx.clone(), seed).await?;

        // Sleep for a while to avoid overwhelming the server
        let sleep_duration = Duration::from_secs(5);
        println!("Sleeping for {:?}", sleep_duration);
        tokio::time::sleep(sleep_duration).await;
    }
    drop(csv_writer_tx);
    writer_handle.await.unwrap();

    // Wait for all tasks to complete
    println!("Load test completed.");
    Ok(())
}
