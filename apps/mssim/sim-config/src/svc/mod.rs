use std::borrow::Cow;

mod call_graph;
mod method_freq;
mod method_latency;

pub use method_latency::MethodLatencyDistMap;

pub type MethodId = Cow<'static, str>;
