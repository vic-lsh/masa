use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[inline]
pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

#[derive(Debug, Clone)]
pub enum LatencyTracker {
    NotStarted,
    Started(Instant),
    Finished(Duration),
}

impl LatencyTracker {
    pub fn start(&mut self) {
        *self = match self {
            Self::NotStarted => Self::Started(Instant::now()),
            _ => panic!("Cannot start tracking latency twice"),
        }
    }

    pub fn record_latency(&mut self) {
        *self = match self {
            Self::Started(inst) => Self::Finished(inst.elapsed()),
            _ => panic!("Cannot record latency if the tracker hasn't started"),
        }
    }

    pub fn get_latency(&self) -> Option<Duration> {
        match self {
            Self::Finished(lat) => Some(*lat),
            _ => None,
        }
    }
}
