use env_logger::{Builder, Env};
use hello::{greeter_client::GreeterClient, HelloRequest};
use log::info;
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Uniform};
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use structopt::StructOpt;
use tonic::metadata::{Context, GlobalGraph};
use tonic::transport::Channel;

pub mod hello {
    tonic::include_proto!("hello");
}
mod graph;

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
        let init_at = time_now();
        let mut rng = StdRng::seed_from_u64(998244353);
        let uniform = Uniform::from(0..1 << 63);
        let mut handles = Vec::with_capacity(self.concurrency);

        for _ in 0..self.concurrency {
            let graph_id = self.global_graph.graph_id().clone();
            let request_id = uniform.sample(&mut rng);
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

                    let start_at = time_now() - init_at;
                    let slo = 10_000;
                    let deadline = start_at + slo;
                    let ctx = Context::new(
                        graph_id.clone(),
                        request_id,
                        start_at,
                        deadline,
                        Some(local_graph.clone()),
                    );
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

fn init_logging() {
    Builder::from_env(Env::default().default_filter_or("info"))
        .format(|buf, record| {
            use std::io::Write;
            writeln!(
                buf,
                "{} [{}:{}] {}",
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                // record.target(),
                record.args()
            )
        })
        .init();
    info!("Logging initialized");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

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

    tokio::time::sleep(Duration::from_secs(1)).await;

    let load_gen = {
        let global_graph = graph::get_global_graph();
        let client = GreeterClient::connect(args.addr).await?;
        let load_gen = LoadGenerator::new(global_graph, client, 1, rps_cnt);
        load_gen
    };
    load_gen.run().await.unwrap();

    Ok(())
}
