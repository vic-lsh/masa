use std::borrow::Cow;

mod method_freq;
mod method_latency;

pub use method_latency::MethodLatencyDistMap;

pub type MethodId = Cow<'static, str>;
