use std::sync::atomic::{AtomicU64, Ordering};
use std::{env, path::Path, sync::Arc, time::Duration};

use masa::{Context as MasaContext, time_now};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::MissedTickBehavior;
use tonic::Request;
use tonic::transport::masa_channel::LoadBalancedChannel;

mod replay;
mod service {
    tonic::include_proto!("service");
}
use replay::{
    QueueLatencySample, ReplayWorkItem, load_frontend_replay_items, queue_latency_output_path,
    run_replay_load, write_queue_latency_csv,
};
use service::RootRequest;
use service::service_client::ServiceClient;
type RpcClient = ServiceClient<LoadBalancedChannel>;

#[derive(Clone)]
enum LoadMode {
    Root,
    Replay {
        work_items: Arc<Vec<ReplayWorkItem>>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = env::var("IP").unwrap_or_else(|_| "[::1]".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "50051".to_string());
    let rps: f64 = env::var("RPS")
        .unwrap_or_else(|_| "400".to_string())
        .parse()?;
    let max_in_flight: usize = env::var("MAX_IN_FLIGHT")
        .unwrap_or_else(|_| "10000".to_string())
        .parse()?;
    let stats_interval_sec: u64 = env::var("STATS_INTERVAL_SEC")
        .unwrap_or_else(|_| "2".to_string())
        .parse()?;

    let replay_env = env::var("REPLAY_TRACE_PATH")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    // If replay_env is set, we are in replay mode
    // Otherwise, we are in root() load mode
    let (load_mode, replay_meta) = match replay_env {
        Some(path_str) => {
            let path = Path::new(&path_str);
            let work_items = load_frontend_replay_items(path)?;
            let count = work_items.len();
            (
                LoadMode::Replay {
                    work_items: Arc::new(work_items),
                },
                Some((path_str, count)),
            )
        }
        None => (LoadMode::Root, None),
    };

    let per_req = if matches!(load_mode, LoadMode::Root) {
        if rps <= 0.0 {
            anyhow::bail!("RPS must be > 0");
        }
        Some(Duration::from_secs_f64(1.0 / rps))
    } else {
        None
    };

    let port = port.parse().unwrap();
    let channel = LoadBalancedChannel::new(addr.clone(), port, 1).await;

    let client = ServiceClient::new(channel);

    match (&load_mode, &replay_meta) {
        (LoadMode::Root, _) => println!("Operating in root() load mode."),
        (LoadMode::Replay { .. }, Some((source, count))) => println!(
            "Operating in replay() load mode with {} requests from {}.",
            count, source
        ),
        (LoadMode::Replay { work_items }, None) => println!(
            "Operating in replay() load mode with {} requests.",
            work_items.len()
        ),
    }

    if per_req.is_some() {
        println!(
            "Starting loadgen: addr={}, rps={}, max_in_flight={}",
            addr, rps, max_in_flight
        );
        println!("Press Ctrl-C to stop.");
    } else if let LoadMode::Replay { work_items } = &load_mode {
        println!(
            "Starting replay: addr={}, requests={}, max_in_flight={}",
            addr,
            work_items.len(),
            max_in_flight
        );
    }

    let sent = Arc::new(AtomicU64::new(0));
    let ok = Arc::new(AtomicU64::new(0));
    let err = Arc::new(AtomicU64::new(0));
    let inflight_guard = Arc::new(Semaphore::new(max_in_flight));
    let queue_samples = Arc::new(Mutex::new(Vec::<QueueLatencySample>::new()));

    let queue_csv_path = replay_meta
        .as_ref()
        .map(|(path_str, _)| queue_latency_output_path(Path::new(path_str.as_str())));

    {
        let sent = sent.clone();
        let ok = ok.clone();
        let err = err.clone();
        tokio::spawn(async move {
            let mut last_sent = 0u64;
            let mut last_ok = 0u64;
            let mut last_err = 0u64;
            let mut ticker = tokio::time::interval(Duration::from_secs(stats_interval_sec));
            loop {
                ticker.tick().await;
                let s = sent.load(Ordering::Relaxed);
                let o = ok.load(Ordering::Relaxed);
                let e = err.load(Ordering::Relaxed);
                println!(
                    "[stats] sent={} (+{}), ok={} (+{}), err={} (+{})",
                    s,
                    s - last_sent,
                    o,
                    o - last_ok,
                    e,
                    e - last_err
                );
                last_sent = s;
                last_ok = o;
                last_err = e;
            }
        });
    }

    match load_mode {
        LoadMode::Root => {
            run_root_load(
                client,
                per_req.expect("per_req available in root mode"),
                sent.clone(),
                ok.clone(),
                err.clone(),
                inflight_guard.clone(),
                max_in_flight,
            )
            .await?;
        }
        LoadMode::Replay { work_items } => {
            run_replay_load(
                client,
                work_items,
                sent.clone(),
                ok.clone(),
                err.clone(),
                inflight_guard.clone(),
                queue_samples.clone(),
            )
            .await?;
        }
    }

    let s = sent.load(Ordering::Relaxed);
    let o = ok.load(Ordering::Relaxed);
    let e = err.load(Ordering::Relaxed);
    println!("Final stats: sent={}, ok={}, err={}", s, o, e);

    if let Some(output_path) = queue_csv_path {
        let samples = {
            let guard = queue_samples.lock().await;
            guard.clone()
        };
        if !samples.is_empty() {
            write_queue_latency_csv(&output_path, &samples).await?;
            println!(
                "Wrote queue latency samples for {} requests to {}",
                samples.len(),
                output_path.display()
            );
        } else {
            println!(
                "No queue latency samples recorded; skipping CSV write to {}",
                output_path.display()
            );
        }
    }

    Ok(())
}

async fn run_root_load(
    client: RpcClient,
    per_req: Duration,
    sent: Arc<AtomicU64>,
    ok: Arc<AtomicU64>,
    err: Arc<AtomicU64>,
    inflight_guard: Arc<Semaphore>,
    max_in_flight: usize,
) -> anyhow::Result<()> {
    let mut ticker = tokio::time::interval(per_req);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.reset();

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                println!("Received Ctrl-C. Shutting down...");
                break;
            }
            _ = ticker.tick() => {
                let permit = match inflight_guard.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                let sent = sent.clone();
                let ok = ok.clone();
                let err = err.clone();
                let mut rpc_client = client.clone();

                sent.fetch_add(1, Ordering::Relaxed);

                tokio::spawn(async move {
                    let _permit = permit;
                    let req_id = sent.load(Ordering::Relaxed);
                    let start_at = time_now();
                    let mut request = Request::new(RootRequest {
                        req_id,
                        start_at,
                    });

                    let ctx = {
                        let slo = 50_000;
                        let start_at = time_now();
                        let deadline = start_at + slo;
                        MasaContext::new("root".to_string(), 0, req_id, slo, 0, start_at, deadline)
                    };
                    request.metadata_mut().insert_ctx("ctx", &ctx);

                    let res = rpc_client.root(request).await;
                    match res {
                        Ok(_) => ok.fetch_add(1, Ordering::Relaxed),
                        Err(_) => err.fetch_add(1, Ordering::Relaxed),
                    };
                });
            }
        }
    }

    let _ = inflight_guard.acquire_many(max_in_flight as u32).await;

    Ok(())
}
