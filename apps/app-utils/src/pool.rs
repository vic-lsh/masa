use async_memcached::Client as McClient;
use std::{
    collections::VecDeque,
    future::Future,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

use tokio::sync::Notify;

#[derive(Default)]
pub struct Pool<T> {
    inner: Mutex<VecDeque<T>>,
    max_size: usize,
    size: AtomicUsize,
    puts: AtomicUsize,
    has_new_item: Notify,
}

impl<T> Pool<T> {
    pub fn new(max_size: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(max_size)),
            max_size,
            size: AtomicUsize::new(0),
            puts: AtomicUsize::new(0),
            has_new_item: Notify::new(),
        }
    }

    pub async fn get_or_create_async<'a, Fut: Future<Output = T>>(
        &'a self,
        factory: impl Fn() -> Fut,
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

    pub async fn try_get_or_create_async<'a, Fut, E>(
        &'a self,
        factory: impl Fn() -> Fut,
    ) -> Result<PoolItemRef<'a, T>, E>
    where
        Fut: Future<Output = Result<T, E>>,
    {
        {
            // fast path
            let mut pool = self.inner.lock().unwrap();
            if let Some(item) = pool.pop_front() {
                return Ok(PoolItemRef::new(item, self));
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
            match factory().await {
                Ok(item) => return Ok(PoolItemRef::new(item, self)),
                Err(err) => {
                    self.size.fetch_sub(1, Ordering::Relaxed);
                    return Err(err);
                }
            }
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
                    return Ok(PoolItemRef::new(item, self));
                }
            }
            // wait for capacity
            self.has_new_item.notified().await;
            iters += 1;
        }
    }

    fn put(&self, item: T) {
        let idle_conns;
        {
            let mut inner = self.inner.lock().unwrap();
            inner.push_back(item);
            idle_conns = inner.len();
        }
        self.has_new_item.notify_waiters();

        if self.puts.fetch_add(1, Ordering::Relaxed) % 2000 == 0 {
            let sz = self.size.load(Ordering::Relaxed);
            log::warn!("pool sz {} idle {}", sz, idle_conns);
        }
    }

    fn release_one(&self) {
        self.size.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

pub struct PoolItemRef<'a, T> {
    // [Note] it is always initialized from the perspective of the users.
    // The field only becomes uninitialized in Drop.
    item: MaybeUninit<T>,
    discarded: bool,
    pool: &'a Pool<T>,
}

impl<'a, T> PoolItemRef<'a, T> {
    // [Note] this is intentionally private to make sure self.item is properly initialized.
    fn new(item: T, pool: &'a Pool<T>) -> Self {
        Self {
            item: MaybeUninit::new(item),
            discarded: false,
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

impl<'a, T> PoolItemRef<'a, T> {
    pub fn discard(mut self) {
        self.discard_impl();
    }

    pub async fn replace<Fut>(mut self, factory: impl Fn() -> Fut) -> PoolItemRef<'a, T>
    where
        Fut: Future<Output = T>,
    {
        // self.discard_impl();
        let new_item = factory().await;

        {
            let new_item = MaybeUninit::new(new_item);
            let old = std::mem::replace(&mut self.item, new_item);
            // SAFETY: self.item is initialized at construction and was never
            // de-initialized.
            unsafe { old.assume_init() }
        };

        self
    }

    fn discard_impl(&mut self) {
        self.discarded = true;
        self.pool.release_one();
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
        if !self.discarded {
            self.pool.put(item);
        }
    }
}

pub struct McPoolItemRef<'a> {
    item: PoolItemRef<'a, McClient>,
    addr: &'a str,
}

impl<'a> McPoolItemRef<'a> {
    pub async fn replace(self) -> McPoolItemRef<'a> {
        let item = self
            .item
            .replace(|| async {
                println!("Making a new McClient");
                McClient::new(&self.addr).await.unwrap()
            })
            .await;
        Self::new(self.addr, item)
    }
}

impl<'a> Deref for McPoolItemRef<'a> {
    type Target = McClient;

    fn deref(&self) -> &Self::Target {
        &*self.item
    }
}

impl<'a> DerefMut for McPoolItemRef<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.item
    }
}

impl<'a> McPoolItemRef<'a> {
    fn new(addr: &'a str, item: PoolItemRef<'a, McClient>) -> Self {
        Self { item, addr }
    }
}

pub struct McPool {
    pool: Pool<McClient>,
    pub addr: String,
}

impl McPool {
    pub fn new(addr: String, max_conns: usize) -> Self {
        log::warn!("MC connecting to {}...", addr);
        Self {
            pool: Pool::new(max_conns),
            addr,
        }
    }

    pub async fn get<'a>(&'a self) -> McPoolItemRef<'a> {
        let pool_item_ref = self
            .pool
            .get_or_create_async(|| async {
                McClient::new(&self.addr)
                    .await
                    .expect(&format!("MC connection to '{}' should succeed", self.addr))
            })
            .await;
        McPoolItemRef::new(self.addr.as_str(), pool_item_ref)
    }
}
