use crossbeam_channel::unbounded;
use rand_distr::{Distribution, Normal};
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Server for benchmarking")]
pub struct Args {}

#[derive(Debug)]
pub struct Request {
    send_at: u64,
    finish_at: u64,
    prev_elapse: u64,
    total_elapse: u64,
    hint: u64,
}

impl Ord for Request {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.hint.cmp(&self.hint)
    }
}

impl PartialOrd for Request {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(other.hint.cmp(&self.hint))
    }
}

impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        self.hint == other.hint
    }
}

impl Eq for Request {}

pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();

    let (tx, rx) = unbounded();

    let client = tokio::spawn(async move {
        for i in 0..10 {
            busy_spin(Duration::from_millis(100));
            let req = Request {
                send_at: time_now(),
                finish_at: 0,
                prev_elapse: 0,
                total_elapse: 0,
                hint: i,
            };
            tx.send(req).unwrap();
        }
    });

    let server = tokio::spawn(async move {
        let mut heap = BinaryHeap::new();
        loop {
            while !rx.is_empty() {
                match rx.try_recv() {
                    Ok(req) => {
                        heap.push(req);
                    }
                    Err(_) => {
                        break;
                    }
                }
            }
            if let Some(req) = heap.pop() {
                busy_spin(Duration::from_millis(500));
                println!("server: {:?}, len: {}", req, heap.len());
            }
        }
    });

    client.await?;
    server.await?;

    Ok(())
}
