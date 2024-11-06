use serde_json;
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
use tonic_masa::{Context, PriorityHint};

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
    ctx: Context,
    latency: u64,
    latency_fe: u64,
    error: String,
}

#[allow(dead_code)]
impl Span {
    pub fn new(ctx: Context, latency: u64, latency_fe: u64, error: String) -> Self {
        Self {
            ctx,
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
        "graph_id,test_id,request_id,slo,request_class,start_at,deadline,latest_exec_at,latency,latency_fe,error"
    )
    .unwrap();
    while let Ok(span) = trace_rx.recv() {
        writeln!(
            file,
            "{},{},{},{},{},{},{},{},{},{},{}",
            span.ctx.graph_id(),
            span.ctx.test_id(),
            span.ctx.request_id(),
            span.ctx.slo(),
            span.ctx.request_class(),
            span.ctx.start_at(),
            span.ctx.deadline(),
            span.ctx.latest_exec_at(),
            span.latency,
            span.latency_fe,
            span.error
        )
        .unwrap();
    }
    log::warn!("Traces fetched");
}

pub struct JsonParser {}

impl JsonParser {
    pub fn new() -> JsonParser {
        JsonParser {}
    }

    pub fn read(&self, path: &str) -> serde_json::Value {
        let data = fs::read_to_string(path).expect("Unable to read file");
        let res: serde_json::Value = serde_json::from_str(&data).expect("Unable to parse");
        res
    }
}
