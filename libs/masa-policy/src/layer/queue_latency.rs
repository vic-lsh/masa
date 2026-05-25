// Queue latency tracking layer — observes queue latencies and propagates
// them through the request tree.
//
// When `trace_queue_latency` is enabled, tracks initial and resume queue
// latencies via the tokio runtime, aggregates child queue latencies from
// responses, and injects the totals into the response context.
//
// When `trace_queue_latency` is disabled, compiles to a zero-size type with
// no-op methods.

use super::LayerServer;

// ── Server ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) struct QueueLatencyServer;

impl QueueLatencyServer {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl LayerServer for QueueLatencyServer {}

// ── Per-Request (trace_queue_latency enabled) ───────────────────────────

#[cfg(feature = "trace_queue_latency")]
mod inner {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    use masa_core::{Context, QueueLatencies};
    use tonic::{CowGrpcMethod, Response, Status};

    use super::super::{Layer, LayerChild};
    use super::QueueLatencyServer;
    use crate::context_ext::MasaResponseExt;

    static SERVICE_NAME: OnceLock<String> = OnceLock::new();

    fn service_name() -> &'static str {
        SERVICE_NAME
            .get_or_init(|| std::env::var("SERVICE_NAME").unwrap_or_else(|_| "unknown".to_string()))
    }

    #[derive(Debug)]
    pub(crate) struct QueueLatencyLayer {
        initial_q_lat: AtomicU64,
        resume_q_lat: AtomicU64,
        is_first_poll: AtomicBool,
        own_queue_len: AtomicU64,
        child_queue_lengths: Mutex<HashMap<String, u64>>,
    }

    impl Layer for QueueLatencyLayer {
        type Server = QueueLatencyServer;
        type Child = QueueLatencyChild;

        fn new(_method: &CowGrpcMethod, _server: &QueueLatencyServer, _ctx: &mut Context) -> Self {
            Self {
                initial_q_lat: AtomicU64::new(0),
                resume_q_lat: AtomicU64::new(0),
                is_first_poll: AtomicBool::new(true),
                own_queue_len: AtomicU64::new(0),
                child_queue_lengths: Mutex::new(HashMap::new()),
            }
        }

        #[inline]
        fn before_poll<Ret>(&self, _ctx: &Context) -> Result<(), Result<Response<Ret>, Status>> {
            let queue_latency = tokio::task::obtain_task_queue_latency().as_micros() as u64;
            if self.is_first_poll.swap(false, Ordering::Relaxed) {
                if queue_latency > 0 {
                    self.initial_q_lat
                        .fetch_add(queue_latency, Ordering::AcqRel);
                }
                let q_len = tokio::runtime::current_thread_queue_len() as u64;
                self.own_queue_len.store(q_len, Ordering::Release);
            } else if queue_latency > 0 {
                self.resume_q_lat.fetch_add(queue_latency, Ordering::AcqRel);
            }
            Ok(())
        }

        #[inline]
        fn after_child_rpc<T>(
            &self,
            _ctx: &Context,
            _child_method: &CowGrpcMethod,
            response: &mut Result<Response<T>, Status>,
            _child_ctx: &QueueLatencyChild,
        ) -> Result<(), Status> {
            if let Ok(resp) = response {
                if let Some(ctx) = resp.get_masa_context() {
                    if let Some(ql) = ctx.queue_latencies {
                        self.initial_q_lat.fetch_add(ql.initial, Ordering::AcqRel);
                        self.resume_q_lat.fetch_add(ql.resume, Ordering::AcqRel);
                        if !ql.queue_lengths.is_empty() {
                            let mut child_qls = self.child_queue_lengths.lock().unwrap();
                            for (svc, len) in ql.queue_lengths {
                                child_qls
                                    .entry(svc)
                                    .and_modify(|e| *e = (*e).max(len))
                                    .or_insert(len);
                            }
                        }
                    }
                }
            }
            Ok(())
        }

        #[inline]
        fn finalize<Ret>(&self, ctx: &mut Context, _result: &mut Result<Response<Ret>, Status>) {
            let initial = self.initial_q_lat.load(Ordering::Acquire);
            let resume = self.resume_q_lat.load(Ordering::Acquire);
            let own_len = self.own_queue_len.load(Ordering::Acquire);
            let mut queue_lengths = std::mem::take(&mut *self.child_queue_lengths.lock().unwrap());
            queue_lengths.insert(service_name().to_string(), own_len);
            ctx.queue_latencies = Some(QueueLatencies {
                initial,
                resume,
                queue_lengths,
            });
        }
    }

    #[derive(Debug, Clone)]
    pub(crate) struct QueueLatencyChild;

    impl LayerChild for QueueLatencyChild {
        fn new() -> Self {
            Self
        }
    }
}

// ── Per-Request (trace_queue_latency disabled) ──────────────────────────

#[cfg(not(feature = "trace_queue_latency"))]
mod inner {
    use masa_core::Context;
    use tonic::CowGrpcMethod;

    use super::super::{Layer, LayerChild};
    use super::QueueLatencyServer;

    #[derive(Debug)]
    pub(crate) struct QueueLatencyLayer;

    impl Layer for QueueLatencyLayer {
        type Server = QueueLatencyServer;
        type Child = QueueLatencyChild;

        fn new(_method: &CowGrpcMethod, _server: &QueueLatencyServer, _ctx: &mut Context) -> Self {
            Self
        }
    }

    #[derive(Debug, Clone)]
    pub(crate) struct QueueLatencyChild;

    impl LayerChild for QueueLatencyChild {
        fn new() -> Self {
            Self
        }
    }
}

pub(crate) use inner::QueueLatencyLayer;
