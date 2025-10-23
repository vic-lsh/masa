use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use masa::{time_now, Context as MasaContext};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::time::Instant;
use tokio::time::MissedTickBehavior;
use tokio::{fs, time};
use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::Request;

mod replay;
use replay::extract_queue_latency;

mod service {
    tonic::include_proto!("service");
}
use replay::{
    load_frontend_replay_items, queue_latency_output_path, run_replay_load,
    write_queue_latency_csv, QueueLatencySample, ReplayWorkItem,
};
use service::service_client::ServiceClient;
use service::RootRequest;
type RpcClient = ServiceClient<LoadBalancedChannel>;

const PERIODIC_FLUSH_INTERVAL_SECS: u64 = 10;
const OUTPUT_DIR: &str = "loadgen_output";

#[derive(Default)]
struct Stats {
    sent: AtomicUsize,
    ok: AtomicUsize,
    err: AtomicUsize,
}

fn root_latency_file_name_for_rps(rps: f64) -> String {
    let mut rps_str = if (rps.fract()).abs() < f64::EPSILON {
        format!("{rps:.0}")
    } else {
        format!("{rps:.2}")
    };
    if rps_str.contains('.') {
        rps_str = rps_str
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();
    }
    format!("root_latencies_{}rps.csv", rps_str.replace('.', "_"))
}

#[derive(Clone)]
enum LoadMode {
    Root,
    Replay {
        work_items: Arc<Vec<ReplayWorkItem>>,
    },
}

async fn run_root_load(
    client: RpcClient,
    per_req: Duration,
    request_slo: u64,
    stats: Arc<Stats>,
    inflight_guard: Arc<Semaphore>,
    max_in_flight: usize,
    root_samples: Arc<Mutex<Vec<RootLatencySample>>>,
    finish_after: Option<Duration>,
    latency_sample_tx: mpsc::UnboundedSender<u64>,
) -> anyhow::Result<()> {
    let mut ticker = tokio::time::interval(per_req);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.reset();

    let finish_deadline = finish_after.map(|duration| Instant::now() + duration);

    let finish_sleep = async {
        if let Some(deadline) = finish_deadline {
            time::sleep_until(deadline).await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::pin!(finish_sleep);

    let shutdown_signal = tokio::signal::ctrl_c();
    tokio::pin!(shutdown_signal);

    loop {
        tokio::select! {
            _ = &mut shutdown_signal => {
                println!("Received Ctrl-C. Shutting down...");
                break;
            }
            _ = &mut finish_sleep => {
                println!("Experiment finished as duration elapsed. Shutting down...");
                break;
            }

            _ = ticker.tick() => {
                let permit = match inflight_guard.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                let mut rpc_client = client.clone();
                let root_samples = root_samples.clone();
                let latency_sample_tx = latency_sample_tx.clone();

                let req_id = stats.sent.fetch_add(1, Ordering::Relaxed) as u64;

                let stats = Arc::clone(&stats);
                tokio::spawn(async move {
                    let _permit = permit;
                    let start_at = time_now();
                    let mut request = Request::new(RootRequest {
                        req_id,
                        start_at,
                    });

                    let ctx = {
                        let slo_us = request_slo * 1000;
                        let start_at = time_now();
                        let deadline = start_at + slo_us;
                        MasaContext::new("root".to_string(), req_id, slo_us, start_at, deadline)
                    };
                    request.metadata_mut().insert_ctx("ctx", &ctx);

                    let start_time = Instant::now();
                    let res = rpc_client.root(request).await;
                    let elapsed = start_time.elapsed().as_micros() as u64;
                    match res {
                        Ok(resp) => {
                            stats.ok.fetch_add(1, Ordering::Relaxed);
                            let queue_latency = extract_queue_latency(resp.metadata());
                            let sample = RootLatencySample {
                                is_err: false,
                                req_id,
                                start_at,
                                queue_latency_us: queue_latency.unwrap_or(0),
                                e2e_latency_us: elapsed,
                            };
                            {
                                let mut guard = root_samples.lock().await;
                                guard.push(sample);
                            }
                            let _ = latency_sample_tx.send(elapsed);
                        }
                        Err(_) => {
                            stats.err.fetch_add(1, Ordering::Relaxed);
                            let sample = RootLatencySample {
                                is_err: true,
                                req_id,
                                start_at,
                                queue_latency_us: 0,
                                e2e_latency_us: 0,
                            };
                            {
                                let mut guard = root_samples.lock().await;
                                guard.push(sample);
                            }
                        }
                    };
                });
            }
        }
    }

    // Drain in-flight requests before exit
    let _ = inflight_guard.acquire_many(max_in_flight as u32).await;

    let s = stats.sent.load(Ordering::Relaxed);
    let o = stats.ok.load(Ordering::Relaxed);
    let e = stats.err.load(Ordering::Relaxed);
    println!("Final stats: sent={}, ok={}, err={}", s, o, e);

    Ok(())
}

#[derive(Debug, Clone)]
struct RootLatencySample {
    req_id: u64,
    start_at: u64,
    queue_latency_us: u64,
    e2e_latency_us: u64,
    is_err: bool,
}

async fn flush_root_samples(
    samples: Arc<Mutex<Vec<RootLatencySample>>>,
    file_name: &str,
) -> anyhow::Result<()> {
    flush_root_samples_internal(samples, file_name, true)
        .await
        .map(|_| ())
}

async fn flush_root_samples_internal(
    samples: Arc<Mutex<Vec<RootLatencySample>>>,
    file_name: &str,
    log_when_empty: bool,
) -> anyhow::Result<Option<usize>> {
    let output_dir = PathBuf::from(OUTPUT_DIR);

    let snapshot = {
        let mut guard = samples.lock().await;
        if guard.is_empty() {
            if log_when_empty {
                println!(
                    "No root() latency samples recorded; skipping CSV write to {}",
                    output_dir.display()
                );
            }
            return Ok(None);
        }
        guard.sort_by_key(|sample| sample.req_id);
        guard.clone()
    };

    fs::create_dir_all(&output_dir).await?;
    let file_path = output_dir.join(file_name);

    let mut csv_data = String::from("req_id,is_err,start_at,queue_latency_us,e2e_latency_us\n");
    for sample in &snapshot {
        csv_data.push_str(&format!(
            "{},{},{},{},{}\n",
            sample.req_id,
            sample.is_err,
            sample.start_at,
            sample.queue_latency_us,
            sample.e2e_latency_us
        ));
    }

    println!(
        "Writing root() latency samples for {} requests to {}",
        snapshot.len(),
        file_path.display()
    );

    fs::write(&file_path, csv_data).await?;
    if log_when_empty {
        println!(
            "Wrote root() latency samples for {} requests to {}",
            snapshot.len(),
            file_path.display()
        );
    }

    Ok(Some(snapshot.len()))
}

async fn flush_queue_samples(
    samples: Arc<Mutex<Vec<QueueLatencySample>>>,
    output_path: &Path,
    log_when_empty: bool,
) -> anyhow::Result<Option<usize>> {
    let snapshot = {
        let guard = samples.lock().await;
        if guard.is_empty() {
            if log_when_empty {
                println!(
                    "No queue latency samples recorded; skipping CSV write to {}",
                    output_path.display()
                );
            }
            return Ok(None);
        }
        guard.clone()
    };

    write_queue_latency_csv(output_path, &snapshot).await?;
    if log_when_empty {
        println!(
            "Wrote queue latency samples for {} requests to {}",
            snapshot.len(),
            output_path.display()
        );
    }

    Ok(Some(snapshot.len()))
}

fn compute_latency_percentiles_us(mut samples: Vec<u64>) -> Option<(u64, u64, u64, u64)> {
    if samples.is_empty() {
        return None;
    }

    samples.sort_unstable();
    let p50 = percentile_from_sorted(&samples, 50.0);
    let p90 = percentile_from_sorted(&samples, 90.0);
    let p95 = percentile_from_sorted(&samples, 95.0);
    let p99 = percentile_from_sorted(&samples, 99.0);
    Some((p50, p90, p95, p99))
}

fn percentile_from_sorted(sorted: &[u64], percentile: f64) -> u64 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }

    let mut rank = (percentile / 100.0 * n as f64).ceil() as usize;
    if rank == 0 {
        rank = 1;
    }
    if rank > n {
        rank = n;
    }

    sorted[rank - 1]
}

async fn flush_queue_latency_samples_task(
    samples: Arc<Mutex<Vec<QueueLatencySample>>>,
    path: PathBuf,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(PERIODIC_FLUSH_INTERVAL_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let should_flush = {
            let guard = samples.lock().await;
            !guard.is_empty()
        };
        if !should_flush {
            continue;
        }
        if let Err(err) = flush_queue_samples(samples.clone(), path.as_path(), false).await {
            eprintln!("Failed to periodically flush queue latency samples: {err:?}");
        }
    }
}

async fn flush_rpc_samples_task(samples: Arc<Mutex<Vec<RootLatencySample>>>, file_name: String) {
    let mut ticker = tokio::time::interval(Duration::from_secs(PERIODIC_FLUSH_INTERVAL_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let should_flush = {
            let guard = samples.lock().await;
            !guard.is_empty()
        };
        if !should_flush {
            continue;
        }
        if let Err(err) =
            flush_root_samples_internal(samples.clone(), file_name.as_ref(), false).await
        {
            eprintln!("Failed to periodically flush root() latency samples: {err:?}");
        }
    }
}

async fn print_stats_task(
    mut latency_rx: UnboundedReceiver<u64>,
    stats_interval: Duration,
    stats: Arc<Stats>,
) {
    let mut last_sent = 0;
    let mut last_ok = 0;
    let mut last_err = 0;
    let mut ticker = tokio::time::interval(stats_interval);
    let mut latency_buffer = Vec::new();
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let s = stats.sent.load(Ordering::Relaxed);
                let o = stats.ok.load(Ordering::Relaxed);
                let e = stats.err.load(Ordering::Relaxed);
                let percentiles = {
                    if latency_buffer.is_empty() {
                        None
                    } else {
                        Some(latency_buffer.clone())
                    }
                    .and_then(compute_latency_percentiles_us)
                };
                let (p50_str, p90_str, p95_str, p99_str) = match percentiles {
                    Some((p50, p90, p95, p99)) => {
                        (format!("{p50}us"), format!("{p90}us"), format!("{p95}us"), format!("{p99}us"))
                    }
                    None => ("n/a".to_string(), "n/a".to_string(), "n/a".to_string(), "n/a".to_string()),
                };
                println!(
                    "[stats] sent={} (+{}), ok={} (+{}), err={} (+{}), p50={}, p90={}, p95={}, p99={}",
                    s,
                    s - last_sent,
                    o,
                    o - last_ok,
                    e,
                    e - last_err,
                    p50_str,
                    p90_str,
                    p95_str,
                    p99_str
                );
                last_sent = s;
                last_ok = o;
                last_err = e;
            }
            maybe_sample = latency_rx.recv() => {
                match maybe_sample {
                    Some(sample) => latency_buffer.push(sample),
                    None => break,
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = env::var("IP").unwrap_or_else(|_| "[::1]".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "50051".to_string());
    let rps: f64 = env::var("RPS")
        .unwrap_or_else(|_| "350".to_string())
        .parse()?;

    // print rps
    println!("RPS set to: {}", rps);
    let max_in_flight: usize = env::var("MAX_IN_FLIGHT")
        .unwrap_or_else(|_| "10000".to_string())
        .parse()?;
    let stats_interval_sec: u64 = env::var("STATS_INTERVAL_SEC")
        .unwrap_or_else(|_| "1".to_string())
        .parse()?;

    let duration: u32 = env::var("DURATION")
        .unwrap_or_else(|_| "60".to_string())
        .parse()?;

    let duration = Duration::from_secs(duration as u64);

    let request_slo = env::var("SLO_MS")
        .unwrap_or_else(|_| "100".to_string())
        .parse::<u64>()?;

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

    let stats = Arc::new(Stats::default());

    let inflight_guard = Arc::new(Semaphore::new(max_in_flight));
    let queue_samples = Arc::new(Mutex::new(Vec::<QueueLatencySample>::new()));
    let (latency_sample_tx, latency_sample_rx) = mpsc::unbounded_channel::<u64>();

    let queue_csv_path = replay_meta
        .as_ref()
        .map(|(path_str, _)| queue_latency_output_path(Path::new(path_str.as_str())));
    let root_samples = Arc::new(Mutex::new(Vec::<RootLatencySample>::new()));
    let root_latency_file_name = root_latency_file_name_for_rps(rps);
    let root_samples_handle = if matches!(load_mode, LoadMode::Root) {
        Some((root_samples.clone(), root_latency_file_name.clone()))
    } else {
        None
    };

    if let Some(ref output_path) = queue_csv_path {
        let samples = queue_samples.clone();
        let path = output_path.clone();
        tokio::spawn(async move { flush_queue_latency_samples_task(samples, path).await });
    }

    if matches!(load_mode, LoadMode::Root) {
        let samples = root_samples.clone();
        let file_name = root_latency_file_name.clone();
        tokio::spawn(async move { flush_rpc_samples_task(samples, file_name).await });
    }

    {
        let stats_interval = Duration::from_secs(stats_interval_sec);
        let stats = Arc::clone(&stats);
        tokio::spawn(async move {
            print_stats_task(latency_sample_rx, stats_interval, stats).await;
        });
    }

    match load_mode {
        LoadMode::Root => {
            run_root_load(
                client,
                per_req.expect("per_req available in root mode"),
                request_slo,
                stats.clone(),
                inflight_guard.clone(),
                max_in_flight,
                root_samples.clone(),
                Some(duration),
                latency_sample_tx.clone(),
            )
            .await?;
        }
        LoadMode::Replay { work_items } => {
            run_replay_load(
                client,
                work_items,
                stats.clone(),
                inflight_guard.clone(),
                queue_samples.clone(),
                latency_sample_tx.clone(),
            )
            .await?;
        }
    }

    let s = stats.sent.load(Ordering::Relaxed);
    let o = stats.ok.load(Ordering::Relaxed);
    let e = stats.err.load(Ordering::Relaxed);
    println!("Final stats: sent={}, ok={}, err={}", s, o, e);

    if let Some(ref output_path) = queue_csv_path {
        flush_queue_samples(queue_samples.clone(), output_path.as_path(), true).await?;
    }

    if let Some((root_samples, file_name)) = root_samples_handle {
        flush_root_samples(root_samples, file_name.as_ref()).await?;
    }

    Ok(())
}
