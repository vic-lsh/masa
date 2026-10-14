//! TailClipper global-arrival-time propagation, independent of Masa budgets.
//! Pair with rpcstack-sched's TailClipper queue (enable `tokio/tailclipper` in the application).
use rpcstack::{ChildState, Extensions, Module, WireIn, WireOut};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tonic::{CowGrpcMethod, Request, Response, Status};

/// Immutable global arrival time. Multiple ingress nodes need synchronized
/// clocks; downstream nodes compare this value without reading their clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailClipperWire {
    pub arrival_ns: u64,
}

impl TailClipperWire {
    /// Stamp a new root request. Zero is reserved for infrastructure scheduling.
    pub fn now() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            arrival_ns: nanos.clamp(1, u64::MAX as u128) as u64,
        }
    }
}

/// ORF metadata module. Invalid metadata rejects the request before its handler.
#[derive(Debug)]
pub struct TailClipperModule {
    arrival: Result<TailClipperWire, Status>,
}

fn annotate<T>(
    request: &mut Request<T>,
    wire: &mut WireOut,
    arrival: TailClipperWire,
) -> Result<(), Status> {
    if arrival.arrival_ns == 0 {
        return Err(Status::invalid_argument("zero TailClipper arrival time"));
    }
    wire.put::<TailClipperModule>(&arrival)
        .map_err(|e| Status::internal(e.to_string()))?;
    request.metadata_mut().insert(
        rpcstack_wire::SCHEDULING_HEADER,
        arrival
            .arrival_ns
            .to_string()
            .parse()
            .map_err(|_| Status::internal("invalid scheduling key"))?,
    );
    Ok(())
}

impl Module for TailClipperModule {
    type Server = ();
    type Wire = TailClipperWire;
    const NAME: &'static str = "tailclipper";

    fn before_root_rpc<T>(
        _: &CowGrpcMethod,
        request: &mut Request<T>,
        wire: &mut WireOut,
        _: &mut ChildState,
    ) -> Result<(), Status> {
        let arrival = WireIn::from_metadata(request.metadata())
            .and_then(|w| w.get::<Self>())
            .map_err(|e| Status::invalid_argument(e.to_string()))?
            .unwrap_or_else(TailClipperWire::now);
        annotate(request, wire, arrival)
    }

    fn new(_: &CowGrpcMethod, _: &(), wire: &WireIn<'_>, _: &mut Extensions) -> Self {
        let arrival = wire
            .get::<Self>()
            .map_err(|e| Status::invalid_argument(e.to_string()))
            .map(|w| w.unwrap_or_else(TailClipperWire::now))
            .and_then(|w| {
                if w.arrival_ns == 0 {
                    Err(Status::invalid_argument("zero TailClipper arrival time"))
                } else {
                    Ok(w)
                }
            });
        Self { arrival }
    }

    fn before_poll<T>(&self, _: &mut Extensions) -> Result<(), Result<Response<T>, Status>> {
        match &self.arrival {
            Ok(w) => {
                tokio::task::reprioritize(tokio::task::TaskPriority::new(w.arrival_ns));
                Ok(())
            }
            Err(status) => Err(Err(status.clone())),
        }
    }

    fn before_child_rpc<T>(
        &self,
        _: &CowGrpcMethod,
        _: &mut ChildState,
        request: &mut Request<T>,
        wire: &mut WireOut,
        _: &mut Extensions,
    ) -> Result<(), Status> {
        annotate(request, wire, self.arrival.clone()?)
    }
}

/// End a cooperative CPU slice while remaining runnable in the ORF queue.
///
/// Tokio's `yield_now` defers the wake until other runnable work drains, which
/// temporarily removes the task from the sharing pool. CPU loops implementing
/// limited processor sharing should use this immediate self-wake instead.
/// Neither operation can interrupt code that does not voluntarily yield.
pub async fn yield_slice() {
    let mut yielded = false;
    std::future::poll_fn(move |cx| {
        if yielded {
            std::task::Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    })
    .await
}
