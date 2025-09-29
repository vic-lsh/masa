use std::sync::atomic::{AtomicU64, Ordering};
use std::{env, fs::File, io::BufReader, path::Path, sync::Arc, time::Duration};

use anyhow::Context;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::time::{Instant, MissedTickBehavior};
use tonic::Request;
use tonic::transport::{Channel, Endpoint};

mod service {
    tonic::include_proto!("service");
}
use service::local_span::SpanType as ProtoSpanType;
use service::service_client::ServiceClient;
use service::{
    ChildSpans as ProtoChildSpans, LocalSpan as ProtoLocalSpan,
    ReplayRequest as ProtoReplayRequest, RootRequest, Span as ProtoSpan,
    span::Kind as ProtoSpanKind,
};

const DEFAULT_REPLAY_PATH: &str =
    "apps/hotel/data/out/queue-experiment01/0/fifo/r1150_Search_frontend_original.json";

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct FrontendSpan {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    latency_us: Option<u64>,
    #[serde(default)]
    service_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    start_timestamp: Option<u64>,
    #[serde(default)]
    spans: Vec<FrontendSpan>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct FrontendRequest {
    service_name: String,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    request_id: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    start_at: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    latency_us: Option<u64>,
    #[serde(default)]
    spans: Vec<FrontendSpan>,
}

#[derive(Clone)]
struct ReplayWorkItem {
    offset_us: u64,
    payload: ProtoReplayRequest,
}

#[derive(Clone)]
enum LoadMode {
    Root,
    Replay {
        work_items: Arc<Vec<ReplayWorkItem>>,
    },
}

fn deserialize_opt_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<Value>::deserialize(deserializer)?;
    match opt {
        Some(Value::Number(num)) => num
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom("expected unsigned integer"))
            .map(Some),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                trimmed
                    .parse::<u64>()
                    .map(Some)
                    .map_err(|e| serde::de::Error::custom(e.to_string()))
            }
        }
        Some(Value::Null) => Ok(None),
        Some(other) => Err(serde::de::Error::custom(format!(
            "unsupported value for u64: {}",
            other
        ))),
        None => Ok(None),
    }
}

fn load_frontend_replay_items(path: &Path) -> anyhow::Result<Vec<ReplayWorkItem>> {
    let file = File::open(path)
        .with_context(|| format!("failed to open frontend replay file {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut requests: Vec<FrontendRequest> = serde_json::from_reader(reader)
        .with_context(|| format!("failed to parse frontend replay file {}", path.display()))?;

    if requests.is_empty() {
        anyhow::bail!(
            "frontend replay file {} contained no entries",
            path.display()
        );
    }

    requests.sort_by_key(|req| req.start_at.unwrap_or(u64::MAX));

    let base_start = requests
        .iter()
        .filter_map(|req| req.start_at)
        .min()
        .context("frontend replay file did not contain start_at values")?;

    let mut work_items = Vec::with_capacity(requests.len());
    for request in requests {
        let FrontendRequest {
            service_name,
            request_id,
            start_at,
            latency_us,
            spans,
        } = request;

        let start_at = match start_at {
            Some(ts) => ts,
            None => continue,
        };

        if spans.is_empty() {
            continue;
        }

        let request_label = request_id.map(|id| id.to_string()).unwrap_or(service_name);

        let (proto_spans, total_span_latency) = convert_spans_to_proto(&spans)
            .with_context(|| format!("failed to convert spans for request {request_label}"))?;

        if proto_spans.is_empty() {
            continue;
        }

        let request_latency = latency_us.unwrap_or(total_span_latency);

        work_items.push(ReplayWorkItem {
            offset_us: start_at.saturating_sub(base_start),
            payload: ProtoReplayRequest {
                request_latency,
                spans: proto_spans,
            },
        });
    }

    if work_items.is_empty() {
        anyhow::bail!(
            "frontend replay file {} did not have usable spans",
            path.display()
        );
    }

    work_items.sort_by_key(|item| item.offset_us);

    Ok(work_items)
}

fn convert_spans_to_proto(spans: &[FrontendSpan]) -> anyhow::Result<(Vec<ProtoSpan>, u64)> {
    let mut proto_spans = Vec::with_capacity(spans.len());
    let mut total_latency = 0u64;

    for span in spans {
        let (proto_span, span_latency) = convert_span_to_proto(span)?;
        total_latency = total_latency.saturating_add(span_latency);
        proto_spans.push(proto_span);
    }

    Ok((proto_spans, total_latency))
}

fn convert_span_to_proto(span: &FrontendSpan) -> anyhow::Result<(ProtoSpan, u64)> {
    match span.kind.as_str() {
        "Compute" => {
            let latency = span.latency_us.unwrap_or(0);
            Ok((
                ProtoSpan {
                    kind: Some(ProtoSpanKind::LocalSpan(ProtoLocalSpan {
                        r#type: ProtoSpanType::Compute as i32,
                        val: latency,
                    })),
                },
                latency,
            ))
        }
        "Block" => {
            let latency = span.latency_us.unwrap_or(0);
            Ok((
                ProtoSpan {
                    kind: Some(ProtoSpanKind::LocalSpan(ProtoLocalSpan {
                        r#type: ProtoSpanType::Block as i32,
                        val: latency,
                    })),
                },
                latency,
            ))
        }
        "ChildCall" => {
            let service_name = span
                .service_name
                .clone()
                .with_context(|| format!("child span missing service_name: {span:?}"))?;

            let (child_spans, child_latency) = convert_spans_to_proto(&span.spans)?;
            let latency = span.latency_us.unwrap_or(child_latency);

            Ok((
                ProtoSpan {
                    kind: Some(ProtoSpanKind::ChildSpans(ProtoChildSpans {
                        name: service_name,
                        spans: child_spans,
                    })),
                },
                latency,
            ))
        }
        other => anyhow::bail!("unsupported span type {other}"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = env::var("IP").unwrap_or_else(|_| "[::1]".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "50051".to_string());
    let rps: f64 = env::var("RPS")
        .unwrap_or_else(|_| "10".to_string())
        .parse()?;
    let max_in_flight: usize = env::var("MAX_IN_FLIGHT")
        .unwrap_or_else(|_| "10000".to_string())
        .parse()?;
    let stats_interval_sec: u64 = env::var("STATS_INTERVAL_SEC")
        .unwrap_or_else(|_| "2".to_string())
        .parse()?;
    let hc_timeout_sec: u64 = env::var("HEALTHCHECK_TIMEOUT_SEC")
        .unwrap_or_else(|_| "180".to_string())
        .parse()?;
    let hc_backoff_ms: u64 = env::var("HEALTHCHECK_BACKOFF_MS")
        .unwrap_or_else(|_| "1000".to_string())
        .parse()?;

    let force_root = false;

    let replay_env = env::var("REPLAY_TRACE_PATH")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let (load_mode, replay_meta) = if force_root {
        (LoadMode::Root, None)
    } else {
        let replay_target = replay_env.or_else(|| {
            let default_path = Path::new(DEFAULT_REPLAY_PATH);
            if default_path.exists() {
                Some(default_path.to_string_lossy().into_owned())
            } else {
                None
            }
        });

        let path_str = match replay_target {
            Some(path) => path,
            None => {
                anyhow::bail!(
                    "Replay mode is default, but no trace file found. Provide REPLAY_TRACE_PATH or place a file at {}.",
                    DEFAULT_REPLAY_PATH
                );
            }
        };

        let path = Path::new(&path_str);
        let work_items = load_frontend_replay_items(path)?;
        let count = work_items.len();
        (
            LoadMode::Replay {
                work_items: Arc::new(work_items),
            },
            Some((path_str, count)),
        )
    };

    let per_req = if matches!(load_mode, LoadMode::Root) {
        if rps <= 0.0 {
            anyhow::bail!("RPS must be > 0");
        }
        Some(Duration::from_secs_f64(1.0 / rps))
    } else {
        None
    };

    let addr = format!("http://{}:{}", addr, port);

    let channel = health_check_connect_and_call(
        &addr,
        Duration::from_secs(hc_timeout_sec),
        Duration::from_millis(hc_backoff_ms),
    )
    .await?;

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
            // run_root_load(
            //     client,
            //     per_req.expect("per_req available in root mode"),
            //     sent.clone(),
            //     ok.clone(),
            //     err.clone(),
            //     inflight_guard.clone(),
            //     max_in_flight,
            // )
            // .await?;
            unimplemented!()
        }
        LoadMode::Replay { work_items } => {
            run_replay_load(
                client,
                work_items,
                sent.clone(),
                ok.clone(),
                err.clone(),
                inflight_guard.clone(),
            )
            .await?;
        }
    }

    let s = sent.load(Ordering::Relaxed);
    let o = ok.load(Ordering::Relaxed);
    let e = err.load(Ordering::Relaxed);
    println!("Final stats: sent={}, ok={}, err={}", s, o, e);

    Ok(())
}

async fn run_root_load(
    client: ServiceClient<Channel>,
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
                    let res = rpc_client.root(Request::new(RootRequest {})).await;
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

async fn run_replay_load(
    client: ServiceClient<Channel>,
    work_items: Arc<Vec<ReplayWorkItem>>,
    sent: Arc<AtomicU64>,
    ok: Arc<AtomicU64>,
    err: Arc<AtomicU64>,
    inflight_guard: Arc<Semaphore>,
) -> anyhow::Result<()> {
    let start_instant = Instant::now();
    let mut handles = Vec::with_capacity(work_items.len());

    for item in work_items.iter().cloned() {
        let schedule_time = start_instant + Duration::from_micros(item.offset_us);
        let permit_pool = inflight_guard.clone();
        let sent = sent.clone();
        let ok = ok.clone();
        let err = err.clone();
        let mut rpc_client = client.clone();

        let handle = tokio::spawn(async move {
            tokio::time::sleep_until(schedule_time).await;
            let permit = match permit_pool.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let _permit = permit;

            sent.fetch_add(1, Ordering::Relaxed);

            let res = rpc_client.replay(Request::new(item.payload)).await;
            match res {
                Ok(_) => ok.fetch_add(1, Ordering::Relaxed),
                Err(_) => err.fetch_add(1, Ordering::Relaxed),
            };
        });

        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}

async fn health_check_connect_and_call(
    addr: &str,
    timeout: Duration,
    backoff: Duration,
) -> anyhow::Result<Channel> {
    println!(
        "Health check: ensuring connectivity and root() response (timeout={}s)...",
        timeout.as_secs()
    );

    let endpoint = Endpoint::from_shared(addr.to_string())?.tcp_nodelay(true);

    let deadline = Instant::now() + timeout;
    loop {
        match endpoint.clone().connect().await {
            Ok(ch) => {
                let mut client = ServiceClient::new(ch.clone());
                match client.root(Request::new(RootRequest {})).await {
                    Ok(_) => {
                        println!("Health check passed: connected and root() responded.");
                        return Ok(ch);
                    }
                    Err(e) => {
                        eprintln!("Health check: root() RPC failed: {e}");
                        if Instant::now() >= deadline {
                            anyhow::bail!(
                                "Health check failed: RPC did not succeed before timeout"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Health check: connect to {} failed: {e}", addr);
                if Instant::now() >= deadline {
                    anyhow::bail!("Health check failed: could not connect before timeout");
                }
            }
        }

        tokio::time::sleep(backoff).await;
    }
}
