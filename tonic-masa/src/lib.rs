mod context;
mod deadline;
mod distribution;
mod tracker;

pub use context::{
    Address, Context, GlobalGraph, GraphID, Latency, LocalGraph, Path, RequestID, Span, Timestamp,
};
pub use deadline::DeadlineHint;
pub use distribution::Distribution;
pub use tracker::LatencyTracker;

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
