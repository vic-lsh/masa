pub mod hello {
    tonic::include_proto!("hello");
}

use std::error::Error;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use env_logger::{Builder, Env};
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Uniform};
use structopt::StructOpt;

use tonic::transport::Channel;
use tonic_masa::{Api, Context};

use hello::{greeter_client::GreeterClient, HelloRequest};

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
    #[structopt(long, default_value = "http://[::1]:50052")]
    pub addr: String,
    #[structopt(long)]
    pub concurrency: usize,
}

#[derive(Debug)]
struct LoadGenerator {
    api: Api,
    client: GreeterClient<Channel>,
    concurrency: usize,
    rps_cnt: Arc<AtomicUsize>,
}

impl LoadGenerator {
    pub fn new(
        api: Api,
        client: GreeterClient<Channel>,
        concurrency: usize,
        rps_cnt: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            api,
            client,
            concurrency,
            rps_cnt,
        }
    }

    async fn run(&self) -> Result<(), Box<dyn Error>> {
        let init_at = time_now();
        let mut handles = Vec::with_capacity(self.concurrency);

        for i in 0..self.concurrency {
            let api = self.api.clone();
            let rps_cnt = self.rps_cnt.clone();
            let mut client = self.client.clone();
            let request = HelloRequest {
                name: "Tonic".into(),
            };

            let h = tokio::spawn(async move {
                let mut rng = StdRng::seed_from_u64(998244353 + i as u64);
                let uniform = Uniform::new(0, 1_000_000_007);
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;

                    rps_cnt.fetch_add(1, Ordering::Relaxed);

                    let request_id = uniform.sample(&mut rng);
                    let start_at = time_now() - init_at;
                    let test_id = 0;
                    let slo = 10_000;
                    let request_class = 0;
                    let deadline = start_at + slo;
                    let latest_exec_at = deadline;

                    let ctx = Context::new(
                        api.clone(),
                        test_id,
                        request_id,
                        slo,
                        request_class,
                        start_at,
                        deadline,
                        latest_exec_at,
                    );
                    let mut request = tonic::Request::new(request.clone());
                    request.metadata_mut().insert_ctx("ctx", &ctx);

                    let _response = client.say_hello(request).await.unwrap();
                    break;
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
    log::info!("Logging initialized");
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
        let api: Api = "/hello.Greeter".to_string();
        let client = GreeterClient::connect(args.addr).await?;
        let load_gen = LoadGenerator::new(api, client, args.concurrency, rps_cnt);
        load_gen
    };
    load_gen.run().await.unwrap();

    Ok(())
}
