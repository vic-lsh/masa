mod context;
mod flag;
mod latency_estimator;
mod priority;
mod timing;
mod typing;

pub use context::FutureSpan;
pub use context::{Context, ContextBuilder};
pub use flag::{
    EARLY_RETURN, FIFO, FIFO_QUEUE_TRACING, FIFO_SPAN_TRACING, PRIO_GLOBAL,
    PRIO_GLOBAL_QUEUE_TRACING, PRIO_LOCAL, PRIO_OLDEST,
};
pub use latency_estimator::{LatencyDistribution, LatencyEstimator, LatencyRms};
pub use priority::{Prioritize, PriorityHint};
pub use timing::{time_now, LatencyTracker};
pub use typing::{Address, Api, Latency, MethodId, RequestId, ServiceId, Timestamp};

// ============================================================================
// Policy System Definitions
// ============================================================================

/// Marker trait for Queue implementations.
pub trait Queue: 'static + Send + Sync {
    fn queue_type() -> QueueType;
}

/// Runtime configuration enum for Queue Discipline
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueType {
    Fifo,
    Prio,       // BinaryHeap
    PrioOldest, // BinaryHeapRoundRobin
}

/// Runtime configuration enum for Deadline Policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlinePolicyType {
    None,
    Local,
    Global,
    Oldest,
}

/// Core Policy trait defining the configuration at compile time.
pub trait Policy: 'static + Send + Sync + Copy {
    type Queue: Queue;
    const EARLY_RETURN: bool;
}

/// A composite policy struct.
/// Q: Queue Type, E: Early Return (bool), P: Deadline Policy Marker
pub struct CompositePolicy<Q, const E: bool, P>(std::marker::PhantomData<(Q, P)>);

impl<Q, const E: bool, P> Clone for CompositePolicy<Q, E, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Q, const E: bool, P> Copy for CompositePolicy<Q, E, P> {}

impl<Q, const E: bool, P> Policy for CompositePolicy<Q, E, P>
where
    Q: Queue,
    P: DeadlinePolicyMarker,
{
    type Queue = Q;
    const EARLY_RETURN: bool = E;
}

// Marker traits for deadline policies
pub trait DeadlinePolicyMarker: 'static + Send + Sync + Copy {}

#[derive(Clone, Copy, Debug)]
pub struct DeadlinePolicyNone;
impl DeadlinePolicyMarker for DeadlinePolicyNone {}

#[derive(Clone, Copy, Debug)]
pub struct DeadlinePolicyLocal;
impl DeadlinePolicyMarker for DeadlinePolicyLocal {}

#[derive(Clone, Copy, Debug)]
pub struct DeadlinePolicyGlobal;
impl DeadlinePolicyMarker for DeadlinePolicyGlobal {}

#[derive(Clone, Copy, Debug)]
pub struct DeadlinePolicyOldest;
impl DeadlinePolicyMarker for DeadlinePolicyOldest {}

// Queue Markers
#[derive(Clone, Copy, Debug)]
pub struct Fifo;
impl Queue for Fifo {
    fn queue_type() -> QueueType {
        QueueType::Fifo
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Prio;
impl Queue for Prio {
    fn queue_type() -> QueueType {
        QueueType::Prio
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PrioOldest;
impl Queue for PrioOldest {
    fn queue_type() -> QueueType {
        QueueType::PrioOldest
    }
}

impl std::str::FromStr for QueueType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "fifo" => Ok(QueueType::Fifo),
            "prio" => Ok(QueueType::Prio),
            "prio_oldest" | "prio-oldest" => Ok(QueueType::PrioOldest),
            _ => Err(format!("Unknown queue type: {}", s)),
        }
    }
}

impl std::str::FromStr for DeadlinePolicyType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(DeadlinePolicyType::None),
            "local" => Ok(DeadlinePolicyType::Local),
            "global" => Ok(DeadlinePolicyType::Global),
            "oldest" => Ok(DeadlinePolicyType::Oldest),
            _ => Err(format!("Unknown deadline policy type: {}", s)),
        }
    }
}
