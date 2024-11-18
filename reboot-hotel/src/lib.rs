use async_memcached::Client as McClient;
use serde_json;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::future::Future;
use std::io::Write;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

use crossbeam_channel::Receiver;
use env_logger::{Builder, Env};

use tonic_masa::Context;

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
pub struct Span {
    ctx: Context,
    latency: u64,
    error: String,
}

#[allow(dead_code)]
impl Span {
    pub fn new(ctx: Context, latency: u64, error: String) -> Self {
        Self {
            ctx,
            latency,
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
        "api,test_id,request_id,slo,request_class,start_at,deadline,latest_exec,latency,error"
    )
    .unwrap();
    while let Ok(span) = trace_rx.recv() {
        writeln!(
            file,
            "{},{},{},{},{},{},{},{},{},{}",
            span.ctx.api(),
            span.ctx.test_id(),
            span.ctx.request_id(),
            span.ctx.slo(),
            span.ctx.request_class(),
            span.ctx.start_at(),
            span.ctx.deadline(),
            span.ctx.latest_exec(),
            span.latency,
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

#[derive(Default)]
pub struct AvgTracker {
    sum: AtomicUsize,
    count: AtomicUsize,
}

impl AvgTracker {
    pub fn track(&self, fanout: usize) {
        self.sum.fetch_add(fanout, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(&self) -> usize {
        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            0
        } else {
            let sum = self.sum.load(Ordering::Relaxed);
            sum / count
        }
    }
}

#[derive(Default)]
pub struct Pool<T> {
    inner: Mutex<VecDeque<T>>,
    max_size: usize,
    size: AtomicUsize,
    has_new_item: Notify,
}

impl<T> Pool<T> {
    pub fn new(max_size: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(max_size)),
            max_size,
            size: AtomicUsize::new(0),
            has_new_item: Notify::new(),
        }
    }

    pub async fn get_or_create_async<'a, Fut: Future<Output = T>>(
        &'a self,
        factory: impl FnOnce() -> Fut,
    ) -> PoolItemRef<'a, T> {
        {
            // fast path
            let mut pool = self.inner.lock().unwrap();
            if let Some(item) = pool.pop_front() {
                return PoolItemRef::new(item, self);
            }
        }

        // slow path
        let should_create = {
            if self.size.load(Ordering::Relaxed) < self.max_size {
                // still have capacity -- try to reserve capacity
                let prev = self.size.fetch_add(1, Ordering::Relaxed);
                if prev > self.max_size {
                    // unlucky -- another thread took our spot and we're at capacity.
                    // cancel out our increment.
                    self.size.fetch_sub(1, Ordering::Relaxed);
                    false
                } else {
                    true
                }
            } else {
                false
            }
        };

        if should_create {
            let item = factory().await;
            return PoolItemRef::new(item, self);
        }

        // the truly slow path -- wait for capacity to show up
        let mut iters = 0;
        let start = std::time::Instant::now();
        loop {
            {
                let mut pool = self.inner.lock().unwrap();
                if let Some(item) = pool.pop_front() {
                    if iters >= 1 {
                        log::debug!("mc waited for {}", start.elapsed().as_micros());
                    }
                    return PoolItemRef::new(item, self);
                }
            }
            // wait for capacity
            self.has_new_item.notified().await;
            iters += 1;
        }
    }

    fn put(&self, item: T) {
        {
            self.inner.lock().unwrap().push_back(item);
        }
        self.has_new_item.notify_waiters();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

pub struct PoolItemRef<'a, T> {
    // [Note] it is always initialized from the perspective of the users.
    // The field only becomes uninitialized in Drop.
    item: MaybeUninit<T>,
    pool: &'a Pool<T>,
}

impl<'a, T> PoolItemRef<'a, T> {
    // [Note] this is intentionally private to make sure self.item is properly initialized.
    fn new(item: T, pool: &'a Pool<T>) -> Self {
        Self {
            item: MaybeUninit::new(item),
            pool,
        }
    }
}

impl<'a, T> Deref for PoolItemRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: self.item is initialized at construction and was never
        // de-initialized.
        unsafe { self.item.assume_init_ref() }
    }
}

impl<'a, T> DerefMut for PoolItemRef<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: self.item is initialized at construction and was never
        // de-initialized.
        unsafe { self.item.assume_init_mut() }
    }
}

impl<'a, T> Drop for PoolItemRef<'a, T> {
    fn drop(&mut self) {
        let item = {
            let uninit = MaybeUninit::uninit();
            let item = std::mem::replace(&mut self.item, uninit);
            // SAFETY: self.item is initialized at construction and was never
            // de-initialized.
            unsafe { item.assume_init() }
        };
        self.pool.put(item)
    }
}

pub struct McPool {
    pool: Pool<McClient>,
    addr: String,
}

impl McPool {
    pub fn new(addr: String, max_conns: usize) -> Self {
        log::warn!("MC connect to {}", addr);
        Self {
            pool: Pool::new(max_conns),
            addr,
        }
    }

    pub async fn get<'a>(&'a self) -> PoolItemRef<'a, McClient> {
        self.pool
            .get_or_create_async(|| async { McClient::new(&self.addr).await.unwrap() })
            .await
    }
}
