use crossbeam_channel::{unbounded, Receiver, Sender};
use hello_world::greeter_client::GreeterClient;
use hello_world::HelloRequest;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Client for benchmarking")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub rps: u64,
    #[structopt(short, long, required = true)]
    pub secs: u64,
    #[structopt(short, long, required = true)]
    pub concurrency: u32,
    #[structopt(short, long, default_value = "http://[::1]:50051")]
    pub addr1: String,
    #[structopt(short, long, default_value = "http://[::1]:50052")]
    pub addr2: String,
    #[structopt(short, long, default_value = "client1.txt")]
    pub output1: String,
    #[structopt(short, long, default_value = "client2.txt")]
    pub output2: String,
}

#[derive(Debug, Clone)]
struct Span {
    send_at: u64,
    recv_at: u64,
}

async fn load_gen(
    rps: u64,
    secs: u64,
    concurrency: u32,
    addr: String,
    trace_tx: Sender<Span>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = GreeterClient::connect(addr).await?;

    let start_at = time_now();
    let pause_at = start_at + secs * 1_000_000;

    let mut handles = Vec::with_capacity(concurrency as usize);
    for _ in 0..concurrency {
        let mut client = client.clone();
        let trace_tx = trace_tx.clone();
        let handle = tokio::spawn(async move {
            let request = HelloRequest {
                name: "Tonic".into(),
            };
            loop {
                let send_at = time_now();
                if send_at > pause_at {
                    break;
                }
                let _ = client
                    .say_hello(tonic::Request::new(request.clone()))
                    .await
                    .unwrap();
                let recv_at = time_now();
                let span = Span { send_at, recv_at };
                trace_tx.send(span).unwrap();
            }
        });
        handles.push(handle);
    }
    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}

async fn fetch_traces(output: String, trace_rx: Receiver<Span>) {
    let path = Path::new(&output);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).expect("Failed to create directory");
        }
    }
    let mut file = File::create(output).expect("Failed to create file");
    while let Ok(span) = trace_rx.recv() {
        let latency = span.recv_at - span.send_at;
        writeln!(file, "{}", latency).expect("Failed to write to file");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();
    eprintln!("args: {:?}", args);

    let (trace_tx, trace_rx) = unbounded();
    let gen1 = tokio::spawn(async move {
        load_gen(args.rps, args.secs, args.concurrency, args.addr1, trace_tx)
            .await
            .unwrap();
    });
    let trace1 = tokio::spawn(async move {
        fetch_traces(args.output1, trace_rx).await;
    });

    let (trace_tx, trace_rx) = unbounded();
    let gen2 = tokio::spawn(async move {
        load_gen(args.rps, args.secs, args.concurrency, args.addr2, trace_tx)
            .await
            .unwrap();
    });
    let trace2 = tokio::spawn(async move {
        fetch_traces(args.output2, trace_rx).await;
    });

    let handles = vec![gen1, trace1, gen2, trace2];
    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}
