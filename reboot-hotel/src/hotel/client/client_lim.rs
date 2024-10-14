pub mod hotel {
    tonic::include_proto!("frontend");
}

use std::error::Error;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Uniform};
use structopt::StructOpt;

use tonic::transport::Channel;
use tonic_masa::{Context, GraphId};

use reboot_hotel::init_logging;

use hotel::{frontend_client::FrontendClient, SearchRequest};

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
    #[structopt(long, default_value = "http://[::1]:8660")]
    pub addr: String,
    #[structopt(long)]
    pub concurrency: usize,
}

#[derive(Debug)]
struct LoadGenerator {
    graph_id: GraphId,
    client: FrontendClient<Channel>,
    concurrency: usize,
    rps_cnt: Arc<AtomicUsize>,
}

impl LoadGenerator {
    pub fn new(
        graph_id: GraphId,
        client: FrontendClient<Channel>,
        concurrency: usize,
        rps_cnt: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            graph_id,
            client,
            concurrency,
            rps_cnt,
        }
    }

    async fn run(&self) -> Result<(), Box<dyn Error>> {
        let init_at = time_now();
        let mut handles = Vec::with_capacity(self.concurrency);

        for i in 0..self.concurrency {
            let graph_id = self.graph_id.clone();
            let rps_cnt = self.rps_cnt.clone();
            let mut client = self.client.clone();
            let request = SearchRequest { ave: 61 };

            let h = tokio::spawn(async move {
                let mut rng = StdRng::seed_from_u64(998244353 + i as u64);
                let uniform = Uniform::new(0, 1_000_000_007);
                loop {
                    rps_cnt.fetch_add(1, Ordering::Relaxed);

                    let request_id = uniform.sample(&mut rng);
                    let start_at = time_now() - init_at;
                    let slo = 10_000;
                    let deadline = start_at + slo;
                    let latest_exec_at = deadline;
                    let request_class = 0;

                    let ctx = Context::new(
                        graph_id.clone(),
                        request_id,
                        deadline,
                        latest_exec_at,
                        request_class,
                    );
                    let mut request = tonic::Request::new(request.clone());
                    request.metadata_mut().insert_ctx("ctx", &ctx);

                    let _response = client.handle_search(request).await.unwrap();
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
            log::warn!("rps: {}", rps);
            prev = now;
        }
    });

    let load_gen = {
        let graph_id: GraphId = "Hotel".to_string();
        let client = FrontendClient::connect(args.addr).await?;
        let load_gen = LoadGenerator::new(graph_id, client, args.concurrency, rps_cnt);
        load_gen
    };
    load_gen.run().await.unwrap();

    Ok(())
}
