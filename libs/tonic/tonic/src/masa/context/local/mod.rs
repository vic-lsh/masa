pub(crate) mod latency_map;
pub(crate) mod local;

#[cfg(feature = "emp_admission")]
pub(crate) mod completion_rate_map;

pub(crate) use latency_map::LatencyMap;
