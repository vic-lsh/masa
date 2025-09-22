use http::Request;
use masa::FutureSpan;
use pin_project::pin_project;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};
use tonic::metadata::MetadataMap;
use tower::{Layer, Service};

pub enum LatencyMetric {
    Queue(String, Duration),
    Response(String, Duration),
}

#[derive(Clone)]
pub struct LatencyLayer {
    tx: mpsc::Sender<LatencyMetric>,
}

impl LatencyLayer {
    pub fn new() -> Self {
        let (tx, mut rx) = mpsc::channel(10000);
        tokio::spawn(async move {
            let mut queue_latencies: HashMap<String, Vec<Duration>> = HashMap::new();
            let mut response_latencies: HashMap<String, Vec<Duration>> = HashMap::new();

            // Map of rpc path to file handle
            let mut file_handles: HashMap<String, File> = HashMap::new();
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            loop {
                tokio::select! {
                    Some(metric) = rx.recv() => {
                        let (path, duration, metric_type) = match metric {
                            LatencyMetric::Queue(path, duration) => (path, duration, "Queue"),
                            LatencyMetric::Response(path, duration) => (path, duration, "Response"),
                        };

                        let fname = format!("{}_latencies.log", path.rsplit('/').next().unwrap());

                        // Check if file already exists
                        if !file_handles.contains_key(&path) {
                            let file_result = tokio::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&fname)
                                .await;

                            match file_result {
                                Ok(file) => {
                                    file_handles.insert(path.clone(), file);
                                }
                                Err(e) => {
                                    log::error!("Failed to open log file for {}: {}", &fname, e);
                                }
                            }
                        }

                        if let Some(file) = file_handles.get_mut(&path) {
                            let log_line = format!("[{}] Latency: {:?}\n", metric_type, duration);
                            if let Err(e) = file.write_all(log_line.as_bytes()).await {
                                eprintln!("[Metrics] ERROR: Failed to write to log {}: {}", fname, e);
                            }
                        }

                        // Store the metric for averaging.
                        if metric_type == "Queue" {
                            queue_latencies.entry(path).or_default().push(duration);
                        } else {
                            response_latencies.entry(path).or_default().push(duration);
                        }
                    }
                    _ = interval.tick() => {
                        // Log the latencies
                        for (path, durations) in &response_latencies {
                            let count = durations.len();
                            if count > 0 {
                                let total: Duration = durations.iter().sum();
                                let avg = Duration::from_secs_f64(total.as_secs_f64() / (count as f64));
                                let avg_in_ms = avg.as_secs_f64() * 1000.0;

                                println!("Service: {:<40} | Avg Response: {:>10.2?} ({} requests)", path, avg_in_ms, count);
                            }
                        }

                        for (path, durations) in &queue_latencies {
                             let count = durations.len();
                             if count > 0 {
                                let total: Duration = durations.iter().sum();
                                let avg = Duration::from_secs_f64(total.as_secs_f64() / (count as f64));
                                let avg_in_ms = avg.as_secs_f64() * 1000.0;

                                println!("Service: {:<40} | Avg Queue:    {:>10.2?} ({} requests)", path, avg_in_ms, count);
                            }
                        }
                        // Reset the latencies
                        queue_latencies.clear();
                        response_latencies.clear();
                    }
                }
            }
        });

        Self { tx }
    }
}

impl<S> Layer<S> for LatencyLayer {
    type Service = LatencyService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LatencyService {
            inner,
            tx: self.tx.clone(),
        }
    }
}

#[derive(Clone)]
pub struct LatencyService<S> {
    inner: S,
    tx: mpsc::Sender<LatencyMetric>,
}

impl<S, B> Service<Request<B>> for LatencyService<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        ResponseFuture {
            rpc_path: req.uri().path().to_string(),
            response_future: self.inner.call(req),
            entry_time: Instant::now(),
            first_poll_time: None,
            tx: self.tx.clone(),
        }
    }
}

#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    response_future: F,
    entry_time: Instant,
    first_poll_time: Option<Instant>,
    rpc_path: String,
    tx: mpsc::Sender<LatencyMetric>,
}

impl<F> Future for ResponseFuture<F>
where
    F: Future,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        if this.first_poll_time.is_none() {
            let now = Instant::now();
            *this.first_poll_time = Some(now);
            let queue_lat = now - *this.entry_time;

            let _ = this
                .tx
                .try_send(LatencyMetric::Queue(this.rpc_path.clone(), queue_lat));
        }

        let poll_result = this.response_future.poll(cx);
        if poll_result.is_ready() {
            let response_lat = this.entry_time.elapsed();
            let _ = this
                .tx
                .try_send(LatencyMetric::Response(this.rpc_path.clone(), response_lat));
        }

        poll_result
    }
}

// Extract latency traces from the response headers
pub fn extract_latency_traces(metadata: &MetadataMap) -> Option<Vec<String>> {
    let header_value = metadata
        .get("X-Latency-Traces")
        .expect("missing X-Latency-Traces header")
        .to_str()
        .unwrap();
    let traces: Vec<FutureSpan> = serde_json::from_str(header_value).ok()?;
    traces
        .iter()
        .map(|span| match span {
            FutureSpan::Compute(duration) => format!("Compute({}us)", duration),
            FutureSpan::Block(duration) => format!("Block({}us)", duration),
            FutureSpan::Queueing(duration) => format!("Queueing({}us)", duration),
        })
        .collect::<Vec<String>>()
        .into()
}
