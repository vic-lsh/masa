use serde_json;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Receiver;
use env_logger::{Builder, Env};

use masa::Context;

pub const USE_SYNTHETIC: bool = if cfg!(feature = "synthetic") {
    true
} else {
    false
};

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

pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RequestStats {
    pub ctx: Context,
    pub latency: u64,
    pub error: String,
}

#[allow(dead_code)]
impl RequestStats {
    pub fn new(ctx: Context, latency: u64, error: String) -> Self {
        Self {
            ctx,
            latency,
            error,
        }
    }
}

#[allow(dead_code)]
pub async fn fetch_traces(output: String, trace_rx: Receiver<RequestStats>) {
    let path = Path::new(&output);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).unwrap();
        }
    }
    let mut file = File::create(output).unwrap();
    writeln!(
        file,
        "api,test_id,request_id,slo,request_class,start_at,deadline,latency,error"
    )
    .unwrap();
    while let Ok(span) = trace_rx.recv() {
        writeln!(
            file,
            "{},{},{},{},{},{},{},{},{}",
            span.ctx.api(),
            span.ctx.test_id(),
            span.ctx.request_id(),
            span.ctx.slo(),
            span.ctx.request_class(),
            span.ctx.start_at(),
            span.ctx.deadline(),
            span.latency,
            span.error
        )
        .unwrap();
    }
    log::warn!("All traces fetched");
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
