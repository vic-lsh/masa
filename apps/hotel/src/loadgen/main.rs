use clap::Parser;
use rand::Rng;
use std::{sync::Arc, time::Instant};
use tokio::{
    sync::Mutex,
    task::JoinSet,
    time::{self, Duration},
};
use tonic::transport::Channel;

use hotel_tonic::frontend::frontend_client::FrontendClient;
use hotel_tonic::frontend::{ReservationRequest, SearchRequest};

mod hotel_tonic {
    pub mod frontend {
        tonic::include_proto!("frontend");
    }
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value = "http://localhost:50051")]
    frontend_addr: String,

    #[arg(long, default_value = "5")]
    rps: usize,

    #[arg(long, default_value = "30")]
    duration_secs: u64,

    #[arg(long, default_value = "both")]
    request_type: String, // "search", "reservation", or "both"
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let client = FrontendClient::connect(args.frontend_addr.clone()).await?;
    let client = Arc::new(Mutex::new(client));
    let start = Instant::now();
    let interval = Duration::from_millis(1000 / args.rps as u64);
    let duration = Duration::from_secs(args.duration_secs);

    println!(
        "🚀 Starting load for {}s at {} RPS...",
        args.duration_secs, args.rps
    );

    let mut tasks = JoinSet::new();
    while Instant::now().duration_since(start) < duration {
        let client = Arc::clone(&client);
        let request_type = args.request_type.clone();

        tasks.spawn(async move {
            match request_type.as_str() {
                "search" => {
                    let _ = send_search(client).await;
                }
                "reservation" => {
                    let _ = send_reservation(client).await;
                }
                "both" => {
                    if rand::thread_rng().gen::<bool>() {
                        let _ = send_search(client).await;
                    } else {
                        let _ = send_reservation(client).await;
                    }
                }
                _ => {
                    eprintln!("Unknown request type: {}", request_type);
                }
            }
        });

        time::sleep(interval).await;
    }

    while let Some(task_result) = tasks.join_next().await {
        if let Err(join_err) = task_result {
            eprintln!("loadgen request task failed: {join_err}");
        }
    }

    Ok(())
}

async fn send_search(
    client: Arc<Mutex<FrontendClient<Channel>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let request = tonic::Request::new(SearchRequest {
        lat: 37.77,
        lon: -122.41,
        in_date: "2025-06-01".to_string(),
        out_date: "2025-06-05".to_string(),
        locale: Some("en".to_string()),
    });

    let start = Instant::now();
    let mut locked = client.lock().await;
    let response = locked.handle_search(request).await;
    let elapsed = start.elapsed().as_millis();

    match response {
        Ok(resp) => {
            println!(
                "[SEARCH] {}ms | Returned {} hotels",
                elapsed,
                resp.get_ref().hotels.len()
            );
        }
        Err(e) => {
            eprintln!("[SEARCH] {}ms | ERROR: {}", elapsed, e);
        }
    }

    Ok(())
}

async fn send_reservation(
    client: Arc<Mutex<FrontendClient<Channel>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let request = tonic::Request::new(ReservationRequest {
        username: "testuser".to_string(),
        password: "testpass".to_string(),
        customer: "Alice".to_string(),
        hotels: vec!["hotel1".to_string(), "hotel2".to_string()],
        in_date: "2025-06-01".to_string(),
        out_date: "2025-06-05".to_string(),
        num_rooms: 1,
    });

    let start = Instant::now();
    let mut locked = client.lock().await;
    let response = locked.handle_reservation(request).await;
    let elapsed = start.elapsed().as_millis();

    match response {
        Ok(resp) => {
            println!(
                "[RESERVE] {}ms | Reserved hotels: {:?}",
                elapsed,
                resp.get_ref().hotels
            );
        }
        Err(e) => {
            eprintln!("[RESERVE] {}ms | ERROR: {}", elapsed, e);
        }
    }

    Ok(())
}
