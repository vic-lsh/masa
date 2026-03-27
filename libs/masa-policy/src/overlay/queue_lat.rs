// Queue latency tracking overlay — observes queue latencies and propagates
// them through the request tree.
//
// When `trace-queue` is enabled, tracks initial and resume queue latencies
// via the tokio runtime, aggregates child queue latencies from responses,
// and injects the totals into the response context.
//
// When `trace-queue` is disabled, compiles to a zero-size type with no-op
// methods.

use super::OverlayServer;

// ── Server ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) struct QueueLatOverlayServer;

impl OverlayServer for QueueLatOverlayServer {
    fn new() -> Self {
        Self
    }
}

// ── Per-Request (trace-queue enabled) ───────────────────────────────────

#[cfg(feature = "trace-queue")]
mod inner {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use masa_core::{Context, QueueLatencies};
    use tonic_core::{CowGrpcMethod, Response, Status};

    use super::super::{Overlay, OverlayChild};
    use super::QueueLatOverlayServer;
    use crate::context_ext::MasaResponseExt;

    #[derive(Debug)]
    pub(crate) struct QueueLatOverlay {
        initial_q_lat: AtomicU64,
        resume_q_lat: AtomicU64,
        is_first_poll: AtomicBool,
    }

    impl Overlay for QueueLatOverlay {
        type Server = QueueLatOverlayServer;
        type Child = QueueLatOverlayChild;

        fn new(
            _method: &CowGrpcMethod,
            _server: &QueueLatOverlayServer,
            _ctx: &mut Context,
        ) -> Self {
            Self {
                initial_q_lat: AtomicU64::new(0),
                resume_q_lat: AtomicU64::new(0),
                is_first_poll: AtomicBool::new(true),
            }
        }

        #[inline]
        fn before_poll<Ret>(&self, _ctx: &Context) -> Result<(), Result<Response<Ret>, Status>> {
            let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
            if queue_latency > 0 {
                if self.is_first_poll.swap(false, Ordering::Relaxed) {
                    self.initial_q_lat
                        .fetch_add(queue_latency, Ordering::AcqRel);
                } else {
                    self.resume_q_lat.fetch_add(queue_latency, Ordering::AcqRel);
                }
            }
            Ok(())
        }

        #[inline]
        fn after_child_rpc<T>(
            &self,
            _ctx: &Context,
            _child_method: &CowGrpcMethod,
            response: &mut Result<Response<T>, Status>,
            _child_ctx: &QueueLatOverlayChild,
        ) -> Result<(), Status> {
            if let Ok(resp) = response {
                if let Some(ctx) = resp.get_masa_context() {
                    if let Some(ql) = ctx.queue_latencies {
                        self.initial_q_lat.fetch_add(ql.initial, Ordering::AcqRel);
                        self.resume_q_lat.fetch_add(ql.resume, Ordering::AcqRel);
                    }
                }
            }
            Ok(())
        }

        #[inline]
        fn finalize<Ret>(&self, ctx: &mut Context, _result: &mut Result<Response<Ret>, Status>) {
            let initial = self.initial_q_lat.load(Ordering::Acquire);
            let resume = self.resume_q_lat.load(Ordering::Acquire);
            ctx.queue_latencies = Some(QueueLatencies { initial, resume });
        }
    }

    #[derive(Debug, Clone)]
    pub(crate) struct QueueLatOverlayChild;

    impl OverlayChild for QueueLatOverlayChild {
        fn new() -> Self {
            Self
        }
    }
}

// ── Per-Request (trace-queue disabled) ──────────────────────────────────

#[cfg(not(feature = "trace-queue"))]
mod inner {
    use masa_core::Context;
    use tonic_core::CowGrpcMethod;

    use super::super::{Overlay, OverlayChild};
    use super::QueueLatOverlayServer;

    #[derive(Debug)]
    pub(crate) struct QueueLatOverlay;

    impl Overlay for QueueLatOverlay {
        type Server = QueueLatOverlayServer;
        type Child = QueueLatOverlayChild;

        fn new(
            _method: &CowGrpcMethod,
            _server: &QueueLatOverlayServer,
            _ctx: &mut Context,
        ) -> Self {
            Self
        }
    }

    #[derive(Debug, Clone)]
    pub(crate) struct QueueLatOverlayChild;

    impl OverlayChild for QueueLatOverlayChild {
        fn new() -> Self {
            Self
        }
    }
}

pub(crate) use inner::QueueLatOverlay;
