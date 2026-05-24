//! Fixed-list load balancing helpers used by Masa transports.

use std::{
    marker::PhantomData,
    task::{Context, Poll},
};

use tower_service::Service;

/// A fixed-list round-robin service balancer.
///
/// This is intentionally smaller than `tower::balance::p2c::Balance`: Masa
/// eagerly connects to a static replica list and only needs deterministic
/// round-robin routing across those connections.
#[allow(missing_debug_implementations)]
pub struct Balance<S, Req> {
    services: Vec<S>,
    next: usize,
    _req: PhantomData<Req>,
}

impl<S, Req> Balance<S, Req> {
    /// Create a new fixed-list balancer.
    pub fn new(list: impl Iterator<Item = S>) -> Self {
        let services = list.collect();
        Self {
            services,
            next: 0,
            _req: PhantomData,
        }
    }
}

impl<S, Req> Service<Req> for Balance<S, Req>
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.services[self.next].poll_ready(cx)
    }

    fn call(&mut self, request: Req) -> Self::Future {
        let r = self.services[self.next].call(request);
        self.next = (self.next + 1) % self.services.len();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::Balance;

    use std::{
        future::{ready, Future, Ready},
        pin::Pin,
        sync::{Arc, Mutex},
        task::{Context, Poll, Wake, Waker},
    };

    use tower_service::Service;

    #[derive(Clone)]
    struct IndexedService {
        id: usize,
        ready_log: Arc<Mutex<Vec<usize>>>,
    }

    impl Service<()> for IndexedService {
        type Response = usize;
        type Error = ();
        type Future = Ready<Result<usize, ()>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.ready_log
                .lock()
                .expect("ready log poisoned")
                .push(self.id);
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _request: ()) -> Self::Future {
            ready(Ok(self.id))
        }
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    #[test]
    fn routes_over_fixed_services_in_round_robin_order() {
        let ready_log = Arc::new(Mutex::new(Vec::new()));
        let services = (0..3).map(|id| IndexedService {
            id,
            ready_log: Arc::clone(&ready_log),
        });
        let mut balance = Balance::new(services);

        let waker = Waker::from(Arc::new(NoopWake));
        let mut cx = Context::from_waker(&waker);

        for expected in [0, 1, 2, 0, 1] {
            assert_eq!(balance.poll_ready(&mut cx), Poll::Ready(Ok(())));

            let mut response = balance.call(());
            assert_eq!(
                Pin::new(&mut response).poll(&mut cx),
                Poll::Ready(Ok(expected))
            );
        }

        let ready_log = ready_log.lock().expect("ready log poisoned");
        assert_eq!(&*ready_log, &[0, 1, 2, 0, 1]);
    }
}
