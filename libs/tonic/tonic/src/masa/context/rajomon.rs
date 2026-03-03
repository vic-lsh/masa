use crate::{CowGrpcMethod, Status};
#[cfg(feature = "rajomon")]
use dashmap::DashMap;
#[cfg(not(feature = "rajomon"))]
use masa_core::Context;
#[cfg(feature = "rajomon")]
use masa_core::LatencyRms;
#[cfg(feature = "rajomon")]
use masa_core::{Context, LatencyEstimator};
#[cfg(feature = "rajomon")]
use once_cell::sync::Lazy;
#[cfg(feature = "rajomon")]
use std::cmp::max;
#[cfg(feature = "rajomon")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "rajomon")]
use std::time::Duration;

#[cfg(feature = "rajomon")]
pub static RAJOMON_STATE: Lazy<RajomonSharedState> = Lazy::new(|| RajomonSharedState::new());

#[cfg(feature = "rajomon")]
#[derive(Debug)]
pub struct RajomonSharedState {
    pub local_prices: DashMap<CowGrpcMethod, u64>,
    pub downstream_prices: DashMap<CowGrpcMethod, u64>,
    pub queue_latencies: DashMap<CowGrpcMethod, Arc<Mutex<LatencyRms>>>,
}

#[cfg(feature = "rajomon")]
impl RajomonSharedState {
    fn new() -> Self {
        let state = Self {
            local_prices: DashMap::new(),
            downstream_prices: DashMap::new(),
            queue_latencies: DashMap::new(),
        };

        // We can't spawn a tokio task inside Lazy::new() unless we are inside a tokio runtime.
        // It's safer to spawn it during the first `check_inbound` or expose an `init` method.
        // Wait, Lazy::new() is just allocating the struct. We can't spawn here.
        state
    }

    fn update_prices(&self) {
        let updates: Vec<(CowGrpcMethod, u64)> = self
            .queue_latencies
            .iter()
            .map(|e| (e.key().clone(), e.value().lock().unwrap().estimate()))
            .collect();

        for (method, estimate) in updates {
            let current_price = self.local_prices.get(&method).map(|v| *v).unwrap_or(1);
            let new_price = if estimate > 5000 {
                // > 5ms
                current_price + 1
            } else {
                max(1, current_price.saturating_sub(1))
            };
            self.local_prices.insert(method, new_price);
        }
    }

    fn log_pricing_tables(&self) {
        if !self.local_prices.is_empty() {
            let parts: Vec<String> = self
                .local_prices
                .iter()
                .map(|e| format!("{}::{}: {}", e.key().service(), e.key().method(), *e.value()))
                .collect();
            log::info!("Rajomon local_prices: {}", parts.join(", "));
        }
        if !self.downstream_prices.is_empty() {
            let parts: Vec<String> = self
                .downstream_prices
                .iter()
                .map(|e| format!("{}::{}: {}", e.key().service(), e.key().method(), *e.value()))
                .collect();
            log::info!("Rajomon downstream_prices: {}", parts.join(", "));
        }
        if !self.queue_latencies.is_empty() {
            let parts: Vec<String> = self
                .queue_latencies
                .iter()
                .map(|e| {
                    format!(
                        "{}::{}: {} us",
                        e.key().service(),
                        e.key().method(),
                        e.value().lock().unwrap().estimate()
                    )
                })
                .collect();
            log::info!("Rajomon queue_latencies: {}", parts.join(", "));
        }
    }

    // Helper to start the background worker once
    pub(super) fn ensure_worker_started() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WORKER_STARTED: AtomicBool = AtomicBool::new(false);
        if !WORKER_STARTED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async {
                let mut interval = tokio::time::interval(Duration::from_millis(100));
                let mut log_tick: u32 = 0;
                loop {
                    interval.tick().await;
                    RAJOMON_STATE.update_prices();
                    log_tick += 1;
                    if log_tick >= 50 {
                        log_tick = 0;
                        RAJOMON_STATE.log_pricing_tables();
                    }
                }
            });
        }
    }
}

#[derive(Debug)]
pub(crate) struct RajomonHandler {
    #[cfg(feature = "rajomon")]
    rpc: CowGrpcMethod,
    #[cfg(feature = "rajomon")]
    should_drop: bool,
}

impl Default for RajomonHandler {
    fn default() -> Self {
        Self {
            #[cfg(feature = "rajomon")]
            rpc: CowGrpcMethod::new("", ""),
            #[cfg(feature = "rajomon")]
            should_drop: false,
        }
    }
}

impl RajomonHandler {
    pub(crate) fn new(rpc: CowGrpcMethod) -> Self {
        #[cfg(feature = "rajomon")]
        {
            RajomonSharedState::ensure_worker_started();
            Self {
                rpc,
                should_drop: false,
            }
        }
        #[cfg(not(feature = "rajomon"))]
        {
            let _ = rpc;
            Self {}
        }
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn check_inbound(&mut self, ctx: &mut Context) -> bool {
        let price = RAJOMON_STATE
            .local_prices
            .get(&self.rpc)
            .map(|v| *v)
            .unwrap_or(1);
        if !ctx.consume_tokens(price) {
            self.should_drop = true;
            true
        } else {
            false
        }
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn check_inbound(&mut self, _ctx: &mut Context) -> bool {
        false
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn should_drop(&self) -> bool {
        self.should_drop
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn should_drop(&self) -> bool {
        false
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn check_outbound(
        &self,
        child_method: &CowGrpcMethod,
        ctx: &Context,
    ) -> Result<(), Status> {
        let price = RAJOMON_STATE
            .downstream_prices
            .get(child_method)
            .map(|v| *v)
            .unwrap_or(1);
        if ctx.tokens() < price {
            Err(self.issue_error(Some(child_method)))
        } else {
            Ok(())
        }
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn check_outbound(
        &self,
        _child_method: &CowGrpcMethod,
        _ctx: &Context,
    ) -> Result<(), Status> {
        Ok(())
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn issue_error(&self, child_method: Option<&CowGrpcMethod>) -> Status {
        let mut msg = format!(
            "/EarlyReturn?src={}::{}",
            self.rpc.service(),
            self.rpc.method()
        );

        if let Some(child) = child_method {
            msg.push_str(&format!(
                "?last_rpc={}::{}",
                child.service(),
                child.method()
            ));
        }

        // Keep "Insufficient Rajomon Tokens" for backward compatibility in assertions
        msg.push_str(" Insufficient Rajomon Tokens");

        Status::resource_exhausted(msg)
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn issue_error(&self, _child_method: Option<&CowGrpcMethod>) -> Status {
        Status::resource_exhausted("Rajomon disabled")
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn track_queue_delay(&self) {
        let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
        let entry = RAJOMON_STATE
            .queue_latencies
            .entry(self.rpc.clone())
            .or_insert_with(|| {
                Arc::new(Mutex::new(LatencyRms::new(
                    (Duration::from_millis(100).as_micros() as u64)
                        .try_into()
                        .unwrap(),
                )))
            });
        entry.lock().unwrap().track(queue_latency);
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn track_queue_delay(&self) {}

    #[cfg(feature = "rajomon")]
    pub(crate) fn update_cache_from_response(
        &self,
        child_method: &CowGrpcMethod,
        metadata: &crate::metadata::MetadataMap,
    ) {
        if let Some(price_header) = metadata.get("x-masa-rajomon-price") {
            if let Ok(price_str) = price_header.to_str() {
                if let Ok(price) = price_str.parse::<u64>() {
                    RAJOMON_STATE
                        .downstream_prices
                        .insert(child_method.clone(), price);
                }
            }
        }
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn update_cache_from_response(
        &self,
        _child_method: &CowGrpcMethod,
        _metadata: &crate::metadata::MetadataMap,
    ) {
    }

    #[cfg(feature = "rajomon")]
    pub(crate) fn inject_price_to_response<T>(
        &self,
        result: &mut Result<crate::Response<T>, Status>,
    ) {
        let price = RAJOMON_STATE
            .local_prices
            .get(&self.rpc)
            .map(|v| *v)
            .unwrap_or(1);
        if let Ok(value) = crate::metadata::MetadataValue::try_from(price.to_string()) {
            match result {
                Ok(resp) => {
                    resp.metadata_mut().insert("x-masa-rajomon-price", value);
                }
                Err(status) => {
                    status.metadata_mut().insert("x-masa-rajomon-price", value);
                }
            }
        }
    }

    #[cfg(not(feature = "rajomon"))]
    pub(crate) fn inject_price_to_response<T>(
        &self,
        _result: &mut Result<crate::Response<T>, Status>,
    ) {
    }
}
