use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use masa::{time_now, Context as MasaContext};
use rand::Rng;
use serde::Deserialize;
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
    load_frontend_replay_items, run_replay_load, ReplayWorkItem,
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

#[derive(Debug, Clone, Deserialize)]
struct FrontendTargetConfig {
    service: String,
    ip: String,
    port: u16,
    replicas: u16,
    graph: String,
    slo_ms: u64,
    probability: f32,
}


#[derive(Clone)]
pub struct ClientEntry {
    pub client: RpcClient,
    pub graph: String,
    pub slo_ms: u64,
    pub probability: f32,
}

pub struct ClientPool {
    clients: Arc<Vec<ClientEntry>>,
}

impl ClientPool {
    pub fn new(entries: Vec<ClientEntry>) -> anyhow::Result<Self> {
        Ok(Self {
            clients: Arc::new(entries),
        })
    }

    pub fn len(&self) -> usize {
        self.clients.len()
    }

    pub fn acquire(&self) -> ClientEntry {
        let mut rng = rand::rng();
        let r: f32 = rng.random();
        let mut cumulative = 0.0;

        for client in self.clients.iter() {
            cumulative += client.probability;
            if r <= cumulative {
                return client.clone();
            }
        }

        // If rounding errors cause r > total sum, return last one.
        self.clients.last().unwrap().clone()
    }
}

async fn run_root_load(
    client_pool: ClientPool,
    per_req: Duration,
    stats: Arc<Stats>,
    inflight_guard: Arc<Semaphore>,
    max_in_flight: usize,
    root_samples: Arc<Mutex<Vec<RootLatencySample>>>,
    warmup_duration: Duration,
    finish_after: Option<Duration>,
    latency_sample_tx: mpsc::UnboundedSender<u64>,
) -> anyhow::Result<()> {
    let mut ticker = tokio::time::interval(per_req);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.reset();

    let run_start = Instant::now();
    let measurement_start = run_start + warmup_duration;
    if !warmup_duration.is_zero() {
        println!(
            "Warmup enabled: suppressing stats for {:.1}s before measurement begins.",
            warmup_duration.as_secs_f64()
        );
    }
    let mut measurement_announced = warmup_duration.is_zero();
    let finish_deadline = finish_after.map(|duration| measurement_start + duration);
    let mut next_req_id: u64 = 0;

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
                // request max-in-flight control
                let permit = match inflight_guard.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                let entry = client_pool.acquire();
                let mut rpc_client = entry.client.clone();
                let graph_hint = entry.graph.clone();
                let root_samples = root_samples.clone();
                let latency_sample_tx = latency_sample_tx.clone();

                let now = Instant::now();
                let measurement_active = now >= measurement_start;
                if measurement_active && !measurement_announced {
                    measurement_announced = true;
                    println!("Warmup complete; starting measurement period.");
                }
                if measurement_active {
                    stats.sent.fetch_add(1, Ordering::Relaxed);
                }
                let req_id = next_req_id;
                next_req_id += 1;
                let record_metrics = measurement_active;

                let stats = Arc::clone(&stats);
                tokio::spawn(async move {
                    let _permit = permit;
                    let start_at = time_now();
                    let mut request = Request::new(RootRequest {
                        req_id,
                        start_at,
                        graph_name: graph_hint,
                    });

                    let ctx = {
                        let slo_us = entry.slo_ms * 1000;
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
                            if record_metrics {
                                stats.ok.fetch_add(1, Ordering::Relaxed);
                                let queue_latency = extract_queue_latency(resp.metadata());
                                let sample = RootLatencySample {
                                    graph: entry.graph,
                                    missed_slo: elapsed > entry.slo_ms * 1000,
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
                        }
                        Err(err) => {
                            eprintln!("Request {} failed: {:?}", req_id, err);
                            if record_metrics {
                                stats.err.fetch_add(1, Ordering::Relaxed);
                                let sample = RootLatencySample {
                                    graph: entry.graph,
                                    missed_slo: false,
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
    graph: String,
    req_id: u64,
    start_at: u64,
    queue_latency_us: u64,
    e2e_latency_us: u64,
    is_err: bool,
    missed_slo: bool,
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

    let mut csv_data = String::from("graph,req_id,is_err,start_at,queue_latency_us,e2e_latency_us, missed_slo\n");
    for sample in &snapshot {
        csv_data.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            sample.graph,
            sample.req_id,
            sample.is_err,
            sample.start_at,
            sample.queue_latency_us,
            sample.e2e_latency_us,
            sample.missed_slo
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

    let warmup_duration = Duration::from_secs(
        env::var("WARMUP_SEC")
            .unwrap_or_else(|_| "0".to_string())
            .parse::<u64>()?,
    );

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

    // Find frontend targets from env
    let front_string = fs::read_to_string("frontend.json").await?;

    let target_configs: Vec<FrontendTargetConfig> = serde_json::from_str(&front_string)?;
    assert!(
        !target_configs.is_empty(),
        "FRONTEND_TARGETS must specify at least one target"
    );

    let target_summary = target_configs
        .iter()
        .map(|cfg| {
            let mut attributes = vec![format!("replicas={}", cfg.replicas)];
            attributes.push(format!("service={}", &cfg.service));
            attributes.push(format!("graph={}", &cfg.graph));
            format!("{}:{} ({})", cfg.ip, cfg.port, attributes.join(", "))
        })
        .collect::<Vec<_>>()
        .join(", ");

    let mut clients = Vec::with_capacity(target_configs.len());
    // Get the indexed clients
    for cfg in target_configs {
        let channel = LoadBalancedChannel::new(cfg.ip.clone(), cfg.port, cfg.replicas as u8).await;

        let entry = ClientEntry {
            client: ServiceClient::new(channel),
            graph: cfg.graph,
            slo_ms: cfg.slo_ms,
            probability: cfg.probability,
        };

        clients.push(entry);
    }
    let client_pool = ClientPool::new(clients)?;

    println!(
        "Configured {} frontend target(s): {}",
        client_pool.len(),
        target_summary
    );

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
            "Starting loadgen: targets={}, rps={}, max_in_flight={}",
            target_summary, rps, max_in_flight
        );
        println!("Press Ctrl-C to stop.");
    } else if let LoadMode::Replay { work_items } = &load_mode {
        println!(
            "Starting replay: targets={}, requests={}, max_in_flight={}",
            target_summary,
            work_items.len(),
            max_in_flight
        );
    }

    let stats = Arc::new(Stats::default());

    let inflight_guard = Arc::new(Semaphore::new(max_in_flight));
    let (latency_sample_tx, latency_sample_rx) = mpsc::unbounded_channel::<u64>();

    let root_samples = Arc::new(Mutex::new(Vec::<RootLatencySample>::new()));
    let root_latency_file_name = root_latency_file_name_for_rps(rps);
    let root_samples_handle = if matches!(load_mode, LoadMode::Root) {
        Some((root_samples.clone(), root_latency_file_name.clone()))
    } else {
        None
    };

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
                client_pool,
                per_req.expect("per_req available in root mode"),
                stats.clone(),
                inflight_guard.clone(),
                max_in_flight,
                root_samples.clone(),
                warmup_duration,
                Some(duration),
                latency_sample_tx.clone(),
            )
            .await?;
        }
        LoadMode::Replay { work_items } => {
            run_replay_load(
                client_pool,
                work_items,
                stats.clone(),
                inflight_guard.clone(),
                latency_sample_tx.clone(),
            )
            .await?;
        }
    }

    let s = stats.sent.load(Ordering::Relaxed);
    let o = stats.ok.load(Ordering::Relaxed);
    let e = stats.err.load(Ordering::Relaxed);
    println!("Final stats: sent={}, ok={}, err={}", s, o, e);

    if let Some((root_samples, file_name)) = root_samples_handle {
        flush_root_samples(root_samples, file_name.as_ref()).await?;
    }

    Ok(())
}
