mod context;
mod flag;
mod header;
mod latency_estimator;
mod priority;
mod timing;
mod typing;

pub use context::FutureSpan;
pub use context::{
    invalid_context_header_metadata_message, Context, ContextBuilder, QueueLatencies, ResponseMeta,
    RootMethod, MISSING_CONTEXT_HEADER_MESSAGE,
};
pub use flag::{
    ABORT_SLACK, ABORT_SLO, RAJOMON, SCHED_FIFO, SCHED_ORACLE, SCHED_PRED, SCHED_SLO,
    SCHED_TAILCLIPPER, SIGNAL_SLACK,
};
pub use header::{read_context, read_context_from_headers, read_priority_from_headers};
pub use latency_estimator::{
    LatencyDistribution, LatencyEstimator, LatencyEwma, LatencyMeanVar, LatencyRms,
};
pub use priority::{Prioritize, PriorityHint};
pub use timing::{time_now, LatencyTracker};
pub use typing::{Address, Api, Latency, MethodId, RequestId, ServiceId, Timestamp};

/// Header key for MASA context.
pub const MASA_CONTEXT_HEADER: &str = "ctx";

/// Oracle-only header carrying perfect child RPC wall-clock work in microseconds.
pub const ORACLE_CHILD_WORK_US_HEADER: &str = "x-masa-oracle-child-work-us";

/// Oracle-only header carrying perfect remaining wall-clock work after the child RPC.
pub const ORACLE_REMAINING_AFTER_US_HEADER: &str = "x-masa-oracle-remaining-after-us";
