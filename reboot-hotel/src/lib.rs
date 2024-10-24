use std::fs::{self, File};
use std::future::Future;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Receiver;
use env_logger::{Builder, Env};
use futures_lite::future;

use hyper::rt::Executor;
use tonic::masa::AsyncTaskMetadata;
use tonic_masa::PriorityHint;

pub fn init_logging() {
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

#[derive(Debug)]
pub struct ExecImpl<'a> {
    ex: &'a smol::Executor<'a, AsyncTaskMetadata>,
}

impl<'a> ExecImpl<'a> {
    pub fn new(ex: &'a smol::Executor<'a, AsyncTaskMetadata>) -> Self {
        Self { ex }
    }

    pub fn spawn<T: Send + 'a>(
        &self,
        future: impl Future<Output = T> + Send + 'a,
    ) -> async_task::Task<T, AsyncTaskMetadata> {
        self.ex.spawn(future)
    }

    pub async fn run(&self) {
        // [NOTE] Only a global queue is used in smol::Executor::tick().
        loop {
            self.ex.tick().await;
            // [NOTE] Yield to tokio runtime.
            future::yield_now().await;
        }
    }
}

impl<'a, F> Executor<F> for ExecImpl<'a>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, fut: F, ddl: PriorityHint) {
        self.ex.spawn_with_prio(fut, ddl).fallible().detach();
    }
}

pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Span {
    test_id: u64,
    request_id: u64,
    graph_id: String,
    slo: u64,
    latency: u64,
    latency_fe: u64,
    error: String,
}

#[allow(dead_code)]
impl Span {
    pub fn new(
        test_id: u64,
        request_id: u64,
        graph_id: String,
        slo: u64,
        latency: u64,
        latency_fe: u64,
        error: String,
    ) -> Self {
        Self {
            test_id,
            request_id,
            graph_id,
            slo,
            latency,
            latency_fe,
            error,
        }
    }
}

#[allow(dead_code)]
pub async fn fetch_traces(output: String, trace_rx: Receiver<Span>) {
    let path = Path::new(&output);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).unwrap();
        }
    }
    let mut file = File::create(output).unwrap();
    writeln!(
        file,
        "test.id,request_id,graph_id,slo,latency,latency_fe,error"
    )
    .unwrap();
    while let Ok(span) = trace_rx.recv() {
        writeln!(
            file,
            "{},{},{},{},{},{},{}",
            span.test_id,
            span.request_id,
            span.graph_id,
            span.slo,
            span.latency,
            span.latency_fe,
            span.error
        )
        .unwrap();
    }
    log::warn!("Traces fetched");
}
