use std::{
    marker::PhantomData,
    task::{Context, Poll},
};

use tower_service::Service;

// minimal reimplementation of crate::balance::p2c::Balance:
// - doesn't support changes to the set of services
// - uses round-robin for load balancing
pub struct Balance<S, Req> {
    services: Vec<S>,
    next: usize,
    _req: PhantomData<Req>,
}

impl<S, Req> Balance<S, Req> {
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
    type Response = <S as Service<Req>>::Response;
    type Error = <S as Service<Req>>::Error;
    type Future = <S as Service<Req>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.services[self.next].poll_ready(cx)
    }

    fn call(&mut self, request: Req) -> Self::Future {
        let r = self.services[self.next].call(request);
        self.next = (self.next + 1) % self.services.len();
        r
    }
}
