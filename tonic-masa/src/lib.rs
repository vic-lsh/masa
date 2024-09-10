mod context;
mod deadline;
mod distribution;
mod graph;
mod span;
mod tracker;
mod typing;

pub use context::Context;
pub use deadline::DeadlineHint;
pub use distribution::Distribution;
pub use graph::{GlobalGraph, GlobalGraphInner, LocalGraph, LocalGraphInner};
pub use span::{Span, SpanInner};
pub use tracker::LatencyTracker;
pub use typing::{Address, GraphID, Latency, Path, RequestID, Timestamp};

pub const QUEUE_EDF: bool = if cfg!(feature = "queue_edf") {
    true
} else {
    false
};

pub const EST_OFFLINE: bool = if cfg!(feature = "est_offline") {
    true
} else {
    false
};

pub const EST_ONLINE: bool = if cfg!(feature = "est_online") {
    true
} else {
    false
};

pub const MOCK_DIST: bool = if cfg!(feature = "mock_dist") {
    true
} else {
    false
};

pub const MOCK_HOTEL: bool = if cfg!(feature = "mock_hotel") {
    true
} else {
    false
};
