//! NSDI '25 §3 policy with isolated service state and unchanged token forwarding.
//! Kept separate from the historical benchmark policy to preserve old runs.
use crate::{ClientTokenBucket, RajomonParams, RajomonWire, CLIENT_TOKEN_BUCKET};
use rand::Rng;
use rpcstack::{
    ChildOutcome, ChildState, Extensions, MissingDependency, Module, ModuleServer, Outcome,
    ServerInit, WireIn, WireOut,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tonic::{CowGrpcMethod, Request, Response, Status};

#[derive(Debug, Default)]
struct Interface {
    own: u64,
    queue_max_us: u64,
    children: HashMap<CowGrpcMethod, u64>,
}
impl Interface {
    fn total(&self) -> u64 {
        self.own
            .saturating_add(self.children.values().copied().max().unwrap_or(0))
    }
}

/// One independent controller per constructed server, not one per process.
#[derive(Debug, Default)]
struct Controller {
    interfaces: Mutex<HashMap<CowGrpcMethod, Interface>>,
    started: AtomicBool,
}
impl Controller {
    fn interfaces(&self) -> MutexGuard<'_, HashMap<CowGrpcMethod, Interface>> {
        self.interfaces.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn total(&self, method: &CowGrpcMethod) -> u64 {
        self.interfaces()
            .entry(method.clone())
            .or_insert_with(|| Interface {
                own: RajomonParams::global().init_price,
                ..Interface::default()
            })
            .total()
    }
    fn observe(&self, method: &CowGrpcMethod, queue_us: u64) {
        let mut interfaces = self.interfaces();
        let i = interfaces.entry(method.clone()).or_default();
        i.queue_max_us = i.queue_max_us.max(queue_us);
    }
    fn learn(&self, parent: &CowGrpcMethod, child: &CowGrpcMethod, price: u64) {
        self.interfaces()
            .entry(parent.clone())
            .or_default()
            .children
            .insert(child.clone(), price);
    }
    fn tick(&self, p: &RajomonParams) {
        for i in self.interfaces().values_mut() {
            let queue = std::mem::take(&mut i.queue_max_us);
            if queue > p.latency_threshold_us {
                let increase = (u128::from(queue - p.latency_threshold_us)
                    * u128::from(p.price_step_up)
                    / 1000)
                    .clamp(1, u64::MAX as u128) as u64;
                i.own = i.own.saturating_add(increase);
            } else if u128::from(queue) * 2 < u128::from(p.latency_threshold_us) {
                i.own = i.own.saturating_sub(1);
            }
        }
    }
    fn start(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(self);
        handle.spawn(async move {
            let period = Duration::from_millis(RajomonParams::global().price_update_rate_ms.max(1));
            loop {
                tokio::time::sleep(period).await;
                let Some(state) = weak.upgrade() else { break };
                state.tick(RajomonParams::global());
            }
        });
    }
}
/// Server-owned controller; cloned requests share it, separate servers do not.
#[derive(Debug, Default)]
pub struct PaperRajomonServer(Arc<Controller>);
impl ModuleServer for PaperRajomonServer {
    fn new(_: &mut ServerInit) -> Result<Self, MissingDependency> {
        Ok(Self::default())
    }
}

/// Complete client/server policy composed using only rpcstack hooks.
/// Shares the `rajomon` wire format with the existing implementation; these
/// two modules are alternatives and must not appear in the same stack.
#[derive(Debug)]
pub struct PaperRajomonModule {
    method: CowGrpcMethod,
    server: Arc<Controller>,
    tokens: Result<u64, Status>,
    admitted: bool,
}
impl Module for PaperRajomonModule {
    type Server = PaperRajomonServer;
    type Wire = RajomonWire;
    const NAME: &'static str = "rajomon";

    fn before_root_rpc<T>(
        method: &CowGrpcMethod,
        request: &mut Request<T>,
        wire: &mut WireOut,
        _: &mut ChildState,
    ) -> Result<(), Status> {
        ClientTokenBucket::ensure_worker_started();
        // Explicit tokens support externally managed clients and deterministic
        // replay. Otherwise the module owns root admission and spending.
        let provided = WireIn::from_metadata(request.metadata())
            .and_then(|w| w.get::<Self>())
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let tokens = match provided {
            Some(w) => w.tokens,
            None => CLIENT_TOKEN_BUCKET
                .try_acquire(method)
                .ok_or_else(|| Status::resource_exhausted("Rajomon root rate limit"))?,
        };
        wire.put::<Self>(&RajomonWire::request(tokens))
            .map_err(|e| Status::internal(e.to_string()))
    }
    fn after_root_rpc<T>(
        method: &CowGrpcMethod,
        outcome: ChildOutcome<'_, T>,
        wire: &WireIn<'_>,
        _: &ChildState,
    ) -> Result<(), Status> {
        if outcome.sent().is_some() {
            if let Some(price) = wire
                .get::<Self>()
                .map_err(|e| Status::internal(e.to_string()))?
                .and_then(|w| w.price)
            {
                CLIENT_TOKEN_BUCKET.update_price(method, price);
            }
        }
        Ok(())
    }
    fn new(
        method: &CowGrpcMethod,
        server: &Self::Server,
        wire: &WireIn<'_>,
        _: &mut Extensions,
    ) -> Self {
        let server = &server.0;
        server.start();
        let tokens = wire
            .get::<Self>()
            .map_err(|e| Status::invalid_argument(e.to_string()))
            .and_then(|w| {
                w.map(|w| w.tokens)
                    .ok_or_else(|| Status::invalid_argument("missing Rajomon tokens"))
            });
        let price = server.total(method);
        let admitted = tokens.as_ref().is_ok_and(|t| *t >= price);
        Self {
            method: method.clone(),
            server: server.clone(),
            tokens,
            admitted,
        }
    }
    fn before_poll<T>(&self, _: &mut Extensions) -> Result<(), Result<Response<T>, Status>> {
        // Runtime queue delay is an observation, never estimated from handler
        // wall time (which includes downstream waits).
        let queue = tokio::task::obtain_task_queue_latency()
            .as_micros()
            .min(u64::MAX as u128) as u64;
        self.server.observe(&self.method, queue);
        if let Err(status) = &self.tokens {
            return Err(Err(status.clone()));
        }
        if !self.admitted {
            return Err(Err(Status::resource_exhausted("Rajomon admission")));
        }
        Ok(())
    }
    fn before_child_rpc<T>(
        &self,
        _: &CowGrpcMethod,
        _: &mut ChildState,
        _: &mut Request<T>,
        wire: &mut WireOut,
        _: &mut Extensions,
    ) -> Result<(), Status> {
        if !self.admitted {
            return Err(Status::resource_exhausted("Rajomon admission"));
        }
        // §3.4/§3.5: identical holdings across all branches and sequential hops.
        wire.put::<Self>(&RajomonWire::request(self.tokens.clone()?))
            .map_err(|e| Status::internal(e.to_string()))
    }
    fn after_child_rpc<T>(
        &self,
        method: &CowGrpcMethod,
        outcome: ChildOutcome<'_, T>,
        wire: &WireIn<'_>,
        _: &ChildState,
        _: &Extensions,
    ) -> Result<(), Status> {
        if outcome.sent().is_some() {
            if let Some(price) = wire
                .get::<Self>()
                .map_err(|e| Status::internal(e.to_string()))?
                .and_then(|w| w.price)
            {
                self.server.learn(&self.method, method, price);
            }
        }
        Ok(())
    }
    fn finalize<T>(
        &self,
        _: &mut Result<Response<T>, Status>,
        _: Outcome<'_>,
        wire: &mut WireOut,
        _: &Extensions,
    ) {
        let frequency = RajomonParams::global().price_freq;
        let price = if frequency > 0 && rand::thread_rng().gen_range(0..frequency) == 0 {
            Some(self.server.total(&self.method))
        } else {
            None
        };
        // Fixed scalar wire fields cannot fail serialization.
        wire.put::<Self>(&RajomonWire {
            tokens: self.tokens.clone().unwrap_or(0),
            price,
        })
        .expect("scalar Rajomon wire");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn method(name: &'static str) -> CowGrpcMethod {
        CowGrpcMethod::new("test", name)
    }
    #[test]
    fn interface_prices_are_isolated_and_follow_proportional_hysteresis() {
        let server = Controller::default();
        let p = RajomonParams::default();
        let a = method("a");
        let b = method("b");
        server.total(&a);
        server.total(&b);
        server.observe(&a, p.latency_threshold_us + 1000);
        server.tick(&p);
        assert_eq!(server.total(&a), p.init_price + p.price_step_up);
        assert_eq!(server.total(&b), p.init_price.saturating_sub(1));
        server.observe(&a, p.latency_threshold_us);
        server.tick(&p);
        assert_eq!(server.total(&a), p.init_price + p.price_step_up);
        server.tick(&p);
        assert_eq!(
            server.total(&a),
            (p.init_price + p.price_step_up).saturating_sub(1)
        );
    }
    #[test]
    fn max_downstream_is_parent_specific_and_can_decrease() {
        let server = Controller::default();
        let a = method("a");
        let b = method("b");
        let x = method("x");
        let y = method("y");
        server.learn(&a, &x, 7);
        server.learn(&a, &y, 8);
        server.learn(&b, &x, 2);
        assert_eq!(server.total(&a), 8);
        assert_eq!(server.total(&b), 2);
        server.learn(&a, &y, 3);
        assert_eq!(server.total(&a), 7);
        assert_eq!(
            Controller::default().total(&a),
            RajomonParams::global().init_price
        );
    }
    #[test]
    fn prices_saturate_instead_of_wrapping() {
        let server = Controller::default();
        let m = method("max");
        server.observe(&m, u64::MAX);
        let p = RajomonParams {
            price_step_up: u64::MAX,
            ..RajomonParams::default()
        };
        server.tick(&p);
        server.learn(&m, &method("child"), 10);
        assert_eq!(server.total(&m), u64::MAX);
    }
    #[test]
    fn fanout_forwards_identical_holdings_and_error_feedback_uses_max() {
        let server = PaperRajomonServer::default();
        let parent_method = method("parent");
        server.0.interfaces().insert(
            parent_method.clone(),
            Interface {
                own: 3,
                ..Interface::default()
            },
        );
        let mut incoming = WireOut::new();
        incoming
            .put::<PaperRajomonModule>(&RajomonWire::request(20))
            .unwrap();
        let mut metadata = tonic::metadata::MetadataMap::new();
        incoming.install(&mut metadata);
        let module = PaperRajomonModule::new(
            &parent_method,
            &server,
            &WireIn::from_metadata(&metadata).unwrap(),
            &mut Extensions::new(),
        );
        assert!(module.admitted);
        for (child_name, price) in [("left", 7), ("right", 8)] {
            let child_method = method(child_name);
            let mut wire = WireOut::new();
            let mut child = ChildState::new();
            module
                .before_child_rpc(
                    &child_method,
                    &mut child,
                    &mut Request::new(()),
                    &mut wire,
                    &mut Extensions::new(),
                )
                .unwrap();
            let mut request_meta = tonic::metadata::MetadataMap::new();
            wire.install(&mut request_meta);
            assert_eq!(
                WireIn::from_metadata(&request_meta)
                    .unwrap()
                    .get::<PaperRajomonModule>()
                    .unwrap()
                    .unwrap()
                    .tokens,
                20
            );
            let mut feedback = WireOut::new();
            feedback
                .put::<PaperRajomonModule>(&RajomonWire::response(20, price))
                .unwrap();
            let mut status = Status::resource_exhausted("overload");
            feedback.install(status.metadata_mut());
            let response: Result<Response<()>, Status> = Err(status);
            let wire = WireIn::from_metadata(response.as_ref().unwrap_err().metadata()).unwrap();
            module
                .after_child_rpc(
                    &child_method,
                    ChildOutcome::Sent(&response),
                    &wire,
                    &child,
                    &Extensions::new(),
                )
                .unwrap();
        }
        assert_eq!(server.0.total(&parent_method), 11); // local 3 + max(7,8)
    }
}
