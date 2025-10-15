use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant as StdInstant},
};

use anyhow::Context;
use masa::{time_now, Context as MasaContext};
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::Instant;
use tonic::metadata::MetadataMap;
use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::Request;

use crate::service::local_span::SpanType as ProtoSpanType;
use crate::service::service_client::ServiceClient;
use crate::service::{
    span::Kind as ProtoSpanKind, ChildSpans as ProtoChildSpans, LocalSpan as ProtoLocalSpan,
    ReplayRequest as ProtoReplayRequest, Span as ProtoSpan,
};
use crate::OUTPUT_DIR;

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
    #[serde(
        default,
        deserialize_with = "deserialize_opt_u64",
        alias = "latency_us"
    )]
    traced_latency_us: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    span_latency_us: Option<u64>,
    #[serde(default)]
    spans: Vec<FrontendSpan>,
}

#[derive(Clone)]
pub struct ReplayWorkItem {
    pub offset_us: u64,
    pub payload: ProtoReplayRequest,
}

#[derive(Debug, Clone)]
pub struct QueueLatencySample {
    pub req_id: u64,
    pub queue_latency_us: u64,
    pub e2e_latency_us: u64,
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

pub fn load_frontend_replay_items(path: &Path) -> anyhow::Result<Vec<ReplayWorkItem>> {
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
            span_latency_us,
            spans,
            ..
        } = request;

        let start_at = match start_at {
            Some(ts) => ts,
            None => continue,
        };

        if spans.is_empty() {
            continue;
        }

        let request_label = request_id
            .clone()
            .map(|id| id.to_string())
            .unwrap_or(service_name);

        let (proto_spans, total_span_latency) = convert_spans_to_proto(&spans)
            .with_context(|| format!("failed to convert spans for request {request_label}"))?;

        if proto_spans.is_empty() {
            continue;
        }

        let exclude_queue_latency = span_latency_us.unwrap_or(total_span_latency);
        let deadline = start_at.saturating_add(exclude_queue_latency);
        work_items.push(ReplayWorkItem {
            offset_us: start_at.saturating_sub(base_start),
            payload: ProtoReplayRequest {
                req_id: request_id.unwrap_or(0),
                exclude_queue_latency,
                slo: 50_000,
                start_at,
                deadline,
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

pub async fn run_replay_load(
    client: ServiceClient<LoadBalancedChannel>,
    work_items: Arc<Vec<ReplayWorkItem>>,
    sent: Arc<AtomicU64>,
    ok: Arc<AtomicU64>,
    err: Arc<AtomicU64>,
    inflight_guard: Arc<Semaphore>,
    queue_samples: Arc<Mutex<Vec<QueueLatencySample>>>,
) -> anyhow::Result<()> {
    let start_instant = Instant::now();
    let mut handles = Vec::with_capacity(work_items.len());

    for item in work_items.iter().cloned() {
        let ReplayWorkItem { offset_us, payload } = item;

        let req_id = payload.req_id;
        let schedule_time = start_instant + Duration::from_micros(offset_us);
        let permit_pool = inflight_guard.clone();
        let sent = sent.clone();
        let ok = ok.clone();
        let err = err.clone();
        let queue_samples = queue_samples.clone();
        let mut rpc_client = client.clone();

        let handle = tokio::spawn(async move {
            tokio::time::sleep_until(schedule_time).await;
            let permit = match permit_pool.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let _permit = permit;

            sent.fetch_add(1, Ordering::Relaxed);
            let mut request = Request::new(payload);

            let ctx = {
                let slo = 50_000;
                let start_at = time_now();
                let deadline = start_at + slo;
                MasaContext::new("replay".to_string(), 0, req_id, slo, 0, start_at, deadline)
            };

            request.metadata_mut().insert_ctx("ctx", &ctx);
            let send_started = StdInstant::now();
            let res = rpc_client.replay(request).await;
            let e2e_latency_us = send_started.elapsed().as_micros().min(u64::MAX as u128) as u64;

            match res {
                Ok(resp) => {
                    if let Some(latency) = extract_queue_latency(resp.metadata()) {
                        let mut samples = queue_samples.lock().await;
                        samples.push(QueueLatencySample {
                            req_id,
                            queue_latency_us: latency,
                            e2e_latency_us,
                        });
                    }
                    ok.fetch_add(1, Ordering::Relaxed)
                }
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

pub fn queue_latency_output_path(trace_path: &Path) -> PathBuf {
    let target_dir = PathBuf::from(OUTPUT_DIR);
    let mut file_name = match trace_path.file_stem() {
        Some(stem) => stem.to_os_string(),
        None => std::ffi::OsString::from("queue_latency"),
    };
    file_name.push("_queue_latency.csv");
    target_dir.join(file_name)
}

pub fn extract_queue_latency(metadata: &MetadataMap) -> Option<u64> {
    metadata
        .get("x-queue-latency")
        .or_else(|| metadata.get("X-Queue-Latency"))
        .and_then(|value| value.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

pub async fn write_queue_latency_csv(
    path: &Path,
    samples: &[QueueLatencySample],
) -> anyhow::Result<()> {
    let mut sorted = samples.to_vec();
    sorted.sort_by_key(|sample| sample.req_id);

    let mut csv_data = String::from("request_id,queue_latency_us,e2e_latency_us\n");
    for sample in sorted {
        csv_data.push_str(&format!(
            "{},{},{}\n",
            sample.req_id, sample.queue_latency_us, sample.e2e_latency_us
        ));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    fs::write(path, csv_data).await?;
    Ok(())
}
